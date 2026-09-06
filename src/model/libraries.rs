use std::{cmp::Ordering, collections::HashSet, io::ErrorKind, path::PathBuf, str::FromStr};

use nanoid::nanoid;
use rs_plugin_common_interfaces::RsRequest;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{create_dir_all, read_dir, remove_dir_all, remove_file, File},
    io::{AsyncReadExt, AsyncWriteExt},
};

use crate::{
    domain::{
        library::{
            LibraryLimits, LibraryMessage, LibraryRole, LibraryStatusMessage, LibraryType,
            ServerLibrary, ServerLibrarySettings, UserMapping,
        },
        ElementAction,
    },
    error::RsResult,
    plugins::sources::{
        error::SourcesError, path_provider::PathProvider, AsyncReadPinBox, FileStreamResult,
        Source, SourceRead,
    },
    server::get_server_file_path_array,
    tools::{
        auth::{sign_local, ClaimsLocal, ClaimsLocalType},
        log::{log_error, log_info, LogServiceType},
    },
};

use super::{
    error::{Error, Result},
    users::{ConnectedUser, UserRole},
    ModelController,
};

// region:    --- Library Role

impl From<u8> for &LibraryRole {
    fn from(level: u8) -> Self {
        if level < 9 {
            return &LibraryRole::None;
        } else if level < 20 {
            return &LibraryRole::Read;
        } else if level < 30 {
            return &LibraryRole::Write;
        } else if level == 254 {
            return &LibraryRole::Admin;
        }
        return &LibraryRole::None;
    }
}
impl From<&LibraryRole> for u8 {
    fn from(role: &LibraryRole) -> Self {
        match role {
            &LibraryRole::Admin => 254,
            &LibraryRole::Write => 20,
            &LibraryRole::Read => 10,
            &LibraryRole::None => 0,
        }
    }
}

impl PartialOrd for LibraryRole {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let a = u8::from(self);
        let b = u8::from(other);
        Some(a.cmp(&b))
    }
}

impl core::fmt::Display for LibraryRole {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            LibraryRole::Admin => write!(f, "admin"),
            LibraryRole::Read => write!(f, "read"),
            LibraryRole::Write => write!(f, "write"),
            LibraryRole::None => write!(f, "none"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerLibraryInvitation {
    pub code: String,
    pub expires: Option<String>,
    pub library: String,
    pub roles: Vec<LibraryRole>,
    pub limits: LibraryLimits,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ServerLibraryForRead {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crypt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<ServerLibrarySettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<LibraryRole>>,
    #[serde(default)]
    #[serde(rename = "passwordProtected")]
    pub password_protected: bool,

    #[serde(default)]
    pub hidden: bool,
}
impl From<ServerLibrary> for ServerLibraryForRead {
    fn from(lib: ServerLibrary) -> Self {
        ServerLibraryForRead {
            id: lib.id,
            name: lib.name,
            source: Some(lib.source),
            root: lib.root,
            kind: lib.kind,
            crypt: lib.crypt,
            settings: Some(lib.settings),
            roles: None,
            password_protected: lib.password.is_some(),

            ..Default::default()
        }
    }
}
impl ServerLibraryForRead {
    pub fn is_virtual(&self) -> bool {
        self.source.as_deref() == Some("virtual")
    }

    fn into_with_role(lib: ServerLibrary, roles: &Vec<LibraryRole>) -> Self {
        ServerLibraryForRead {
            id: lib.id,
            name: lib.name,
            source: Some(lib.source),
            root: lib.root,
            kind: lib.kind,
            crypt: lib.crypt,
            settings: Some(lib.settings),
            roles: Some(roles.to_owned()),
            password_protected: lib.password.is_some(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerLibraryForUpdate {
    pub name: Option<String>,
    pub source: Option<String>,
    pub root: Option<String>,
    pub settings: Option<ServerLibrarySettings>,
    pub credentials: Option<String>,
    pub plugin: Option<String>,
    #[serde(
        rename = "password",
        default,
        skip_serializing,
        deserialize_with = "reject_password_update"
    )]
    _password: Option<()>,
}

fn reject_password_update<'de, D>(deserializer: D) -> std::result::Result<Option<()>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let _ = serde::de::IgnoredAny::deserialize(deserializer)?;
    Err(<D::Error as serde::de::Error>::custom(
        "password changes must use the library encryption endpoint",
    ))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEncryptionRequest {
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEncryptionStatus {
    pub phase: String,
    pub processed_items: u64,
    pub total_items: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub password_protected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_password_protected: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct LibraryEncryptionJob {
    pub id: String,
    pub library_id: String,
    pub source_password: Option<String>,
    pub target_password: Option<String>,
    pub phase: String,
    pub snapshot_complete: bool,
    pub total_items: u64,
    pub completed_items: u64,
    pub last_error: Option<String>,
}

impl LibraryEncryptionJob {
    pub fn is_active(&self) -> bool {
        self.phase == "running"
    }
}

#[derive(Debug, Clone)]
pub struct LibraryEncryptionItem {
    pub id: String,
    pub job_id: String,
    pub kind: String,
    pub media_id: Option<String>,
    pub source: String,
    pub staged_source: Option<String>,
    pub state: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerLibraryForAdd {
    pub name: String,
    pub source: String,
    pub root: Option<String>,
    pub settings: ServerLibrarySettings,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    pub crypt: Option<bool>,
    pub credentials: Option<String>,
    pub plugin: Option<String>,
    pub password: Option<String>,
}

pub(super) fn map_library_for_user(
    library: ServerLibrary,
    user: &ConnectedUser,
) -> Option<ServerLibraryForRead> {
    let hidden = user.has_hidden_library(library.id.clone());
    match user {
        ConnectedUser::Server(user) => {
            let rights = user.libraries.iter().find(|x| x.id == library.id);
            if let Some(rights) = rights {
                let mut library_out = ServerLibraryForRead::into_with_role(library, &rights.roles);
                library_out.hidden = hidden;
                if !rights.has_role(&LibraryRole::Admin) {
                    library_out.root = None;
                    library_out.settings = None;
                }
                Some(library_out)
            } else {
                None
            }
        }
        ConnectedUser::Anonymous | ConnectedUser::Guest(_) => None,
        ConnectedUser::ServerAdmin => Some(ServerLibraryForRead::from(library)),
        ConnectedUser::Share(claims) => {
            if claims.kind == ClaimsLocalType::Admin {
                Some(ServerLibraryForRead::from(library))
            } else {
                None
            }
        }
        ConnectedUser::UploadKey(key) => {
            let mut library_out =
                ServerLibraryForRead::into_with_role(library, &vec![LibraryRole::Write]);
            if library_out.id == key.library {
                library_out.root = None;
                library_out.settings = None;
                Some(library_out)
            } else {
                None
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySocketMessage {
    pub action: ElementAction,
    pub library: ServerLibraryForRead,
}

impl LibraryMessage {
    pub fn for_socket(&self, user: &ConnectedUser) -> Option<LibrarySocketMessage> {
        if let Some(library) = map_library_for_user(self.library.clone(), user) {
            Some(LibrarySocketMessage {
                action: self.action.clone(),
                library,
            })
        } else {
            None
        }
    }
}

fn merged_progress_users(mappings: &[UserMapping], user_id: &str) -> HashSet<String> {
    let mut users = HashSet::from([user_id.to_string()]);
    let directly_connected = mappings
        .iter()
        .filter(|mapping| mapping.to == user_id || mapping.from == user_id);

    for connected in directly_connected {
        users.insert(connected.from.clone());
        users.insert(connected.to.clone());

        for mapping in mappings.iter().filter(|mapping| {
            mapping.to == connected.to
                || mapping.from == connected.from
                || mapping.to == connected.from
                || mapping.from == connected.to
        }) {
            users.insert(mapping.from.clone());
            users.insert(mapping.to.clone());
        }
    }

    users
}

impl ModelController {
    pub async fn get_library(
        &self,
        library_id: &str,
        requesting_user: &ConnectedUser,
    ) -> Result<Option<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Read)?;
        let lib = self.store.get_library(library_id).await?;
        if let Some(lib) = lib {
            let return_library = map_library_for_user(lib, &requesting_user);
            Ok(return_library)
        } else {
            Ok(None)
        }
    }

    pub async fn get_internal_library(
        &self,
        library_id: &str,
    ) -> Result<Option<super::libraries::ServerLibrary>> {
        let lib = self.store.get_library(library_id).await?;
        Ok(lib)
    }

    pub async fn get_libraries(
        &self,
        requesting_user: &ConnectedUser,
    ) -> Result<Vec<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_role(&UserRole::Read)?;
        let libraries = self
            .store
            .get_libraries()
            .await?
            .into_iter()
            .flat_map(|l| map_library_for_user(l, &requesting_user));
        Ok(libraries.collect::<Vec<super::libraries::ServerLibraryForRead>>())
    }

    pub async fn get_library_mapped_users(&self, library_id: &str) -> Result<Vec<UserMapping>> {
        let library = self
            .get_library(library_id, &ConnectedUser::ServerAdmin)
            .await?
            .ok_or(SourcesError::UnableToFindLibrary(
                library_id.to_string(),
                "get_library_mapped_users".to_string(),
            ))?;

        return Ok(library
            .settings
            .and_then(|s| s.map_progress)
            .unwrap_or_default());
    }

    /// Get list of user id this user is currently mapped from
    /// Exemple if user A is mapped to B then passing user A will return B  
    pub async fn get_library_progress_user_mappings(
        &self,
        library_id: &str,
        user_id: String,
    ) -> Result<Vec<String>> {
        let mut mappings = vec![];
        let library = self.get_internal_library(library_id).await?.ok_or(
            SourcesError::UnableToFindLibrary(
                library_id.to_string(),
                "get_library_progress_user_mappings".to_string(),
            ),
        )?;
        if let Some(mapping) = library.settings.map_progress {
            let filtered = mapping.into_iter().filter(|m| m.from == user_id);
            for mapping in filtered {
                mappings.push(mapping.to);
            }
        }
        return Ok(mappings);
    }

    /// Get list of user id this user is currently mapped to
    /// Exemple if user A is mapped to B then passing user B will return A  
    pub async fn get_library_progress_user_mapped(
        &self,
        library_id: &str,
        user_id: String,
    ) -> Result<Vec<String>> {
        let mut mappings = vec![];
        let library = self.get_internal_library(library_id).await?.ok_or(
            SourcesError::UnableToFindLibrary(
                library_id.to_string(),
                "get_library_mapped_users".to_string(),
            ),
        )?;
        if let Some(mapping) = library.settings.map_progress {
            let filtered = mapping.into_iter().filter(|m| m.to == user_id);
            for mapping in filtered {
                mappings.push(mapping.from);
            }
        }
        return Ok(mappings);
    }

    /// Get users that are either side of a mapping with `user_id`, including
    /// `user_id` itself and users mapped to those directly connected users.
    pub async fn get_library_progress_merged_users(
        &self,
        library_id: &str,
        user_id: String,
    ) -> Result<HashSet<String>> {
        let library = self.get_internal_library(library_id).await?.ok_or(
            SourcesError::UnableToFindLibrary(
                library_id.to_string(),
                "get_library_mapped_users".to_string(),
            ),
        )?;
        Ok(merged_progress_users(
            library.settings.map_progress.as_deref().unwrap_or_default(),
            &user_id,
        ))
    }

    pub async fn get_library_mapped_user(
        &self,
        library_id: &str,
        mut user_id: String,
    ) -> Result<String> {
        let library = self.get_internal_library(library_id).await?.ok_or(
            SourcesError::UnableToFindLibrary(
                library_id.to_string(),
                "get_library_mapped_users".to_string(),
            ),
        )?;
        if let Some(mapping) = library.settings.map_progress {
            if let Some(mapping) = mapping.into_iter().find(|m| m.from == user_id) {
                user_id = mapping.to;
            }
        }
        return Ok(user_id);
    }
    /// If library_id is None, return user_id unchanged
    /// If library_id is Some, return mapped user if any, else user_id unchanged
    pub async fn get_optional_library_mapped_user(
        &self,
        library_id: Option<&str>,
        mut user_id: String,
    ) -> Result<String> {
        if let Some(library_id) = library_id {
            let library = self.get_internal_library(library_id).await?.ok_or(
                SourcesError::UnableToFindLibrary(
                    library_id.to_string(),
                    "get_library_mapped_users".to_string(),
                ),
            )?;
            if let Some(mapping) = library.settings.map_progress {
                if let Some(mapping) = mapping.into_iter().find(|m| m.from == user_id) {
                    user_id = mapping.to;
                }
            }
            return Ok(user_id);
        }
        Ok(user_id)
    }

    pub async fn update_library(
        &self,
        library_id: &str,
        update: ServerLibraryForUpdate,
        requesting_user: &ConnectedUser,
    ) -> Result<Option<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
        let _migration_guard = self.library_encryption_gate.read().await;
        self.ensure_library_encryption_writable(library_id).await?;

        self.store.update_library(library_id, update).await?;
        let library = self.store.get_library(library_id).await?;
        if let Some(library) = library {
            self.cache_update_library(library.clone()).await;
            self.send_library(LibraryMessage {
                action: crate::domain::ElementAction::Updated,
                library: library.clone(),
            });

            Ok(map_library_for_user(library, &requesting_user))
        } else {
            Ok(None)
        }
    }

    pub async fn get_library_encryption_status(
        &self,
        library_id: &str,
        requesting_user: &ConnectedUser,
    ) -> Result<LibraryEncryptionStatus> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let library = self
            .get_internal_library(library_id)
            .await?
            .ok_or(Error::LibraryNotFound(library_id.to_string()))?;
        let job = self.store.get_library_encryption_job(library_id).await?;
        Ok(match job {
            Some(job) => LibraryEncryptionStatus {
                phase: job.phase.clone(),
                processed_items: job.completed_items,
                total_items: job.total_items,
                last_error: job.last_error.clone(),
                password_protected: library.password.is_some(),
                target_password_protected: job.is_active().then_some(job.target_password.is_some()),
            },
            None => LibraryEncryptionStatus {
                phase: "idle".to_string(),
                processed_items: 0,
                total_items: 0,
                last_error: None,
                password_protected: library.password.is_some(),
                target_password_protected: None,
            },
        })
    }

    pub async fn request_library_encryption_change(
        &self,
        library_id: &str,
        target_password: Option<String>,
        requesting_user: &ConnectedUser,
    ) -> Result<LibraryEncryptionStatus> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        if target_password.as_deref().is_some_and(str::is_empty) {
            return Err(Error::InvalidLibraryEncryptionRequest(
                "Password must not be empty; use DELETE to remove encryption".to_string(),
            ));
        }

        let _migration_guard = self.library_encryption_gate.write().await;
        if self.deleting_libraries.read().await.contains(library_id) {
            return Err(Error::LibraryDeletionInProgress(library_id.to_string()));
        }
        if self
            .store
            .get_library_encryption_job(library_id)
            .await?
            .is_some_and(|job| job.is_active())
        {
            return Err(Error::LibraryEncryptionInProgress(library_id.to_string()));
        }

        let library = self
            .get_internal_library(library_id)
            .await?
            .ok_or(Error::LibraryNotFound(library_id.to_string()))?;
        if library.crypt.unwrap_or(false) {
            return Err(Error::InvalidLibraryEncryptionRequest(
                "Server password encryption cannot be changed while legacy client encryption is enabled"
                    .to_string(),
            ));
        }
        if library.source == "virtual" {
            return Err(Error::InvalidLibraryEncryptionRequest(
                "Virtual libraries do not contain files to encrypt".to_string(),
            ));
        }
        if library.password == target_password {
            return Err(Error::InvalidLibraryEncryptionRequest(
                "The requested password state is already active".to_string(),
            ));
        }

        let job = LibraryEncryptionJob {
            id: nanoid!(),
            library_id: library_id.to_string(),
            source_password: library.password.clone(),
            target_password: target_password.clone(),
            phase: "running".to_string(),
            snapshot_complete: false,
            total_items: 0,
            completed_items: 0,
            last_error: None,
        };
        self.store
            .create_library_encryption_job(job.clone())
            .await?;
        self.schedule_library_encryption_job(&job)
            .await
            .map_err(|error| Error::Other(error.to_string()))?;

        Ok(LibraryEncryptionStatus {
            phase: job.phase,
            processed_items: 0,
            total_items: 0,
            last_error: None,
            password_protected: library.password.is_some(),
            target_password_protected: Some(target_password.is_some()),
        })
    }

    async fn schedule_library_encryption_job(&self, job: &LibraryEncryptionJob) -> RsResult<()> {
        use crate::tools::scheduler::{
            encrypt_library::EncryptLibraryTask, RsSchedulerWhen, RsTaskType,
        };
        self.scheduler
            .add(
                RsTaskType::EncryptLibrary,
                RsSchedulerWhen::At(0),
                EncryptLibraryTask::new(job.id.clone(), job.library_id.clone()),
            )
            .await
    }

    pub async fn initialize_library_encryption_jobs(&self) -> RsResult<()> {
        for job in self.store.list_active_library_encryption_jobs().await? {
            self.schedule_library_encryption_job(&job).await?;
        }
        Ok(())
    }

    pub async fn add_library(
        &self,
        library_for_add: ServerLibraryForAdd,
        importData: Option<Vec<u8>>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_role(&UserRole::Admin)?;
        let library_id = nanoid!();
        let source = if library_for_add.kind == LibraryType::Iptv {
            "virtual".to_string()
        } else {
            library_for_add.source
        };
        let library = ServerLibrary {
            id: library_id.clone(),
            name: library_for_add.name,
            source,
            root: library_for_add.root,
            kind: library_for_add.kind,
            crypt: library_for_add.crypt,
            settings: library_for_add.settings,
            plugin: library_for_add.plugin,
            credentials: library_for_add.credentials,
            password: library_for_add.password,

            ..Default::default()
        };
        self.store.add_library(library).await?;
        let user_id = requesting_user.user_id()?;
        self.store
            .add_library_rights(
                library_id.clone(),
                user_id,
                vec![LibraryRole::Admin],
                LibraryLimits::default(),
            )
            .await?;
        let library = self
            .store
            .get_library(&library_id)
            .await?
            .ok_or(crate::Error::Error(format!(
                "unable to load librarary from database after creation"
            )))?;

        if let Some(importData) = importData {
            let server_db_path =
                get_server_file_path_array(vec![&"dbs", &format!("db-{}.db", &library.id)])
                    .await
                    .map_err(|_| Error::CannotOpenDatabase)?;
            // Create and write to the file asynchronously
            let mut file = File::create(&server_db_path)
                .await
                .map_err(|_| crate::Error::Error(format!("Failed to create database file")))?;
            file.write_all(&importData)
                .await
                .map_err(|_| crate::Error::Error(format!("Failed to write database file")))?;
            file.flush()
                .await
                .map_err(|_| crate::Error::Error(format!("Failed to flush database file")))?;
        }

        log_info(
            LogServiceType::LibraryCreation,
            format!("Will do first init of library {}", library.name),
        );
        self.cache_update_library(library.clone()).await;

        let source = self.source_for_library(&library.id).await.map_err(|e| {
            Error::ServiceError(
                "Unable to get library source after init".to_string(),
                Some(e.to_string()),
            )
        })?;
        let inited = source.init().await;
        if let Err(err) = inited {
            return Err(Error::ServiceError(
                "Unable to init library source".to_string(),
                Some(err.to_string()),
            )
            .into());
        }

        self.store
            .add_library_to_store(&library_id)
            .await
            .map_err(|e| {
                Error::ServiceError(
                    "Unable to add library to store".to_string(),
                    Some(e.to_string()),
                )
            })?;
        self.send_library(LibraryMessage {
            action: crate::domain::ElementAction::Added,
            library: library.clone(),
        });
        Ok(Some(ServerLibraryForRead::from(library)))
    }

    pub async fn remove_library(
        &self,
        library_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<ServerLibraryForRead> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
        let _migration_guard = self.library_encryption_gate.read().await;
        self.ensure_library_encryption_writable(library_id).await?;
        let library =
            self.store
                .get_library(&library_id)
                .await?
                .ok_or(SourcesError::UnableToFindLibrary(
                    library_id.to_string(),
                    "get_library_mapped_users".to_string(),
                ))?;

        self.cache_remove_library(&library.id).await;
        self.store.remove_library(library_id.to_string()).await?;
        self.send_library(LibraryMessage {
            action: crate::domain::ElementAction::Deleted,
            library: library.clone(),
        });
        Ok(ServerLibraryForRead::from(library))
    }

    pub async fn request_remove_library(
        &self,
        library_id: &str,
        delete_media_content: bool,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        requesting_user.check_role(&UserRole::Admin)?;
        let _migration_guard = self.library_encryption_gate.read().await;
        self.ensure_library_encryption_writable(library_id).await?;
        let library_id = library_id.to_string();

        {
            let mut deleting = self.deleting_libraries.write().await;
            if deleting.contains(&library_id) {
                return Err(Error::LibraryDeletionInProgress(library_id).into());
            }
            deleting.insert(library_id.clone());
        }

        let mc = self.clone();
        tokio::spawn(async move {
            let result = mc
                .remove_library_background(&library_id, delete_media_content)
                .await;
            if let Err(error) = result {
                log_error(
                    LogServiceType::LibraryCreation,
                    format!("Library deletion failed for {}: {:#}", library_id, error),
                );
                mc.send_library_status(LibraryStatusMessage {
                    message: format!("delete-failed: {}", error),
                    library: library_id.clone(),
                });
            }
            mc.deleting_libraries.write().await.remove(&library_id);
        });

        Ok(())
    }

    async fn remove_library_background(
        &self,
        library_id: &str,
        delete_media_content: bool,
    ) -> RsResult<()> {
        let library =
            self.store
                .get_library(library_id)
                .await?
                .ok_or(SourcesError::UnableToFindLibrary(
                    library_id.to_string(),
                    "remove_library_background".to_string(),
                ))?;

        self.send_library_status(LibraryStatusMessage {
            message: "delete-started".to_string(),
            library: library_id.to_string(),
        });

        if delete_media_content {
            self.remove_tracked_media_sources(library_id).await?;
        }

        self.send_library_status(LibraryStatusMessage {
            message: "delete-cleaning-local-cache".to_string(),
            library: library_id.to_string(),
        });

        let local = self.library_source_for_library(library_id).await?;
        let local_root = local.get_full_path("");
        Self::remove_directory_if_exists(&local_root).await?;

        self.cache_remove_library(library_id).await;
        self.store.remove_library(library_id.to_string()).await?;
        self.store.remove_library_from_store(library_id)?;

        self.send_library_status(LibraryStatusMessage {
            message: "delete-cleaning-database-files".to_string(),
            library: library_id.to_string(),
        });
        self.remove_library_database_files(library_id).await?;
        self.send_library(LibraryMessage {
            action: crate::domain::ElementAction::Deleted,
            library,
        });
        self.send_library_status(LibraryStatusMessage {
            message: "delete-completed".to_string(),
            library: library_id.to_string(),
        });

        Ok(())
    }

    async fn remove_tracked_media_sources(&self, library_id: &str) -> RsResult<()> {
        self.send_library_status(LibraryStatusMessage {
            message: "delete-removing-tracked-media".to_string(),
            library: library_id.to_string(),
        });
        let store = self.store.get_library_store(library_id)?;
        let sources = store.get_all_sources().await?;
        let source_provider = self.source_for_library(library_id).await?;
        let total = sources.len();
        for (index, source) in sources.iter().enumerate() {
            if index % 100 == 0 || index + 1 == total {
                self.send_library_status(LibraryStatusMessage {
                    message: format!("delete-media-progress:{}/{}", index + 1, total),
                    library: library_id.to_string(),
                });
            }
            match source_provider.remove(source).await {
                Ok(_) => {}
                Err(crate::Error::Source(SourcesError::NotFound(_))) => {}
                Err(error) => {
                    return Err(Error::ServiceError(
                        "Unable to delete tracked media file while removing library".to_string(),
                        Some(format!("source={} error={}", source, error)),
                    )
                    .into());
                }
            }
        }
        Ok(())
    }

    async fn remove_library_database_files(&self, library_id: &str) -> RsResult<()> {
        let db_path =
            get_server_file_path_array(vec!["dbs", &format!("db-{}.db", library_id)]).await?;
        let db_wal_path = PathBuf::from(format!("{}-wal", db_path.to_string_lossy()));
        let db_shm_path = PathBuf::from(format!("{}-shm", db_path.to_string_lossy()));

        Self::remove_file_if_exists(&db_path).await?;
        Self::remove_file_if_exists(&db_wal_path).await?;
        Self::remove_file_if_exists(&db_shm_path).await?;
        Ok(())
    }

    async fn remove_file_if_exists(path: &PathBuf) -> RsResult<()> {
        match remove_file(path).await {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn remove_directory_if_exists(path: &PathBuf) -> RsResult<()> {
        match remove_dir_all(path).await {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn clean_library(
        &self,
        library_id: &str,
        requesting_user: &ConnectedUser,
    ) -> crate::error::Result<Vec<(String, u64)>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
        let m = self.source_for_library(library_id).await?;
        let store = self.store.get_library_store(library_id)?;
        let sources = store.get_all_sources().await?;
        println!("sources count: {}", sources.len());
        let cleaned = m.clean(sources).await?;

        let local = self.library_source_for_library(library_id).await?;

        local.clean_temp()?;
        Ok(cleaned)
    }

    pub async fn add_library_invitation(
        &self,
        library_id: &str,
        roles: Vec<LibraryRole>,
        limits: LibraryLimits,
        requesting_user: &ConnectedUser,
    ) -> Result<super::libraries::ServerLibraryInvitation> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let invitation = ServerLibraryInvitation {
            code: nanoid!(),
            expires: None,
            library: library_id.to_string(),
            roles,
            limits,
        };
        self.store
            .add_library_invitation(invitation.clone())
            .await?;
        Ok(invitation)
    }

    pub async fn get_watermarks(
        &self,
        library_id: &str,
        requesting_user: &ConnectedUser,
    ) -> Result<Vec<String>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Read)?;
        let local = self.library_source_for_library(library_id).await?;
        let path = local.get_full_path("");
        let mut files = read_dir(&path).await?;
        let mut watermars: Vec<String> = vec![];
        while let Ok(Some(entry)) = files.next_entry().await {
            let metadata = entry.metadata().await?;
            if metadata.is_file() {
                if let Some(filename) = entry.file_name().to_str() {
                    if filename.starts_with(".watermark.") && filename.ends_with(".png") {
                        watermars.push(filename.replace(".watermark.", "").replace(".png", ""));
                    }
                }
            }
        }

        Ok(watermars)
    }

    pub async fn get_watermark(
        &self,
        library_id: &str,
        watermark: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<SourceRead> {
        requesting_user.check_library_role(&library_id, LibraryRole::Read)?;

        let watermark = if watermark == "default" {
            "".to_owned()
        } else {
            format!(".{}", watermark)
        };
        let local = self.library_source_for_library(library_id).await?;
        let sourceread = local
            .get_file(&format!(".watermark{}.png", watermark), None)
            .await?;

        Ok(sourceread)
    }

    pub async fn get_request_share_token(
        &self,
        library_id: &str,
        request: &RsRequest,
        delay_in_seconds: u64,
        requesting_user: &ConnectedUser,
    ) -> Result<String> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let exp = ClaimsLocal::generate_seconds(delay_in_seconds);
        let claims = ClaimsLocal {
            cr: "service::share_request".to_string(),
            kind: crate::tools::auth::ClaimsLocalType::RequestUrl(request.url.to_string()),
            exp,
        };
        let token = sign_local(claims)
            .await
            .map_err(|_| Error::UnableToSignShareToken)?;
        Ok(token)
    }
}

impl ModelController {
    pub async fn request_to_source(
        &self,
        library_id: &str,
        request: RsRequest,
        requesting_user: &ConnectedUser,
    ) -> RsResult<SourceRead> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        Ok(SourceRead::Request(request))
    }

    pub async fn request_to_reader(
        &self,
        library_id: &str,
        request: RsRequest,
        requesting_user: &ConnectedUser,
    ) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let source = self
            .request_to_source(library_id, request, requesting_user)
            .await?;

        let reader = source
            .into_reader(
                Some(library_id),
                None,
                None,
                Some((self.clone(), requesting_user)),
                None,
            )
            .await?;

        Ok(reader)
    }

    pub async fn url_to_source(
        &self,
        library_id: &str,
        url: String,
        requesting_user: &ConnectedUser,
    ) -> RsResult<SourceRead> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;

        let request = RsRequest {
            url,
            ..Default::default()
        };
        let source = SourceRead::Request(request);

        Ok(source)
    }

    pub async fn url_to_reader(
        &self,
        library_id: &str,
        url: String,
        requesting_user: &ConnectedUser,
    ) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        let request = RsRequest {
            url,
            ..Default::default()
        };
        self.request_to_reader(library_id, request, requesting_user)
            .await
    }

    pub async fn url_to_bufer(
        &self,
        library_id: &str,
        url: String,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<u8>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let mut reader = self.url_to_reader(library_id, url, requesting_user).await?;
        // Create a buffer to hold the data
        let mut buffer = Vec::new();

        // Read the entire file into the buffer
        reader.stream.read_to_end(&mut buffer).await?;

        Ok(buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role() {
        assert_eq!(LibraryRole::Read < LibraryRole::Write, true);
        assert_eq!(LibraryRole::Write < LibraryRole::Admin, true);
        assert_eq!(LibraryRole::None < LibraryRole::Read, true);
        assert_eq!(LibraryRole::Admin > LibraryRole::Write, true);
        assert_eq!(LibraryRole::Write > LibraryRole::Read, true);
        assert_eq!(LibraryRole::Read > LibraryRole::None, true);

        assert_eq!(LibraryRole::Read > LibraryRole::Write, false);
    }

    #[test]
    fn merged_progress_users_includes_requester_without_mappings() {
        assert_eq!(
            merged_progress_users(&[], "user-a"),
            HashSet::from(["user-a".to_string()])
        );
    }

    #[test]
    fn merged_progress_users_keeps_existing_mapping_expansion() {
        let mappings = vec![
            UserMapping {
                from: "user-a".to_string(),
                to: "user-b".to_string(),
            },
            UserMapping {
                from: "user-c".to_string(),
                to: "user-b".to_string(),
            },
            UserMapping {
                from: "unrelated-a".to_string(),
                to: "unrelated-b".to_string(),
            },
        ];

        assert_eq!(
            merged_progress_users(&mappings, "user-a"),
            HashSet::from([
                "user-a".to_string(),
                "user-b".to_string(),
                "user-c".to_string(),
            ])
        );
    }

    #[test]
    fn library_reads_expose_only_password_protection_state() {
        let library = ServerLibrary {
            password: Some("do-not-serialize".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_value(ServerLibraryForRead::from(library)).unwrap();

        assert_eq!(json["passwordProtected"], true);
        assert!(json.get("password").is_none());
        assert!(json.get("password_protected").is_none());
    }

    #[test]
    fn ordinary_library_updates_reject_password_fields() {
        let result = serde_json::from_value::<ServerLibraryForUpdate>(serde_json::json!({
            "name": "Renamed",
            "password": "must-use-the-encryption-endpoint"
        }));

        assert!(result.is_err());
    }

    #[test]
    fn ordinary_library_updates_keep_ignoring_unrelated_fields() {
        let result = serde_json::from_value::<ServerLibraryForUpdate>(serde_json::json!({
            "id": "library-1",
            "name": "Renamed",
            "passwordProtected": true
        }));

        assert!(result.is_ok());
        assert_eq!(result.unwrap().name.as_deref(), Some("Renamed"));
    }
}
