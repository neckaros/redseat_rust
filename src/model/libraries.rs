use std::{cmp::Ordering, collections::HashSet, str::FromStr};

use nanoid::nanoid;
use rs_plugin_common_interfaces::RsRequest;
use serde::{Deserialize, Serialize};
use tokio::{fs::{create_dir_all, read_dir, File}, io::{AsyncReadExt, AsyncWriteExt}};

use crate::{domain::{library::{LibraryLimits, LibraryMessage, LibraryRole, LibraryType, ServerLibrary, ServerLibrarySettings, UserMapping}, ElementAction}, error::RsResult, plugins::sources::{error::SourcesError, path_provider::PathProvider, AsyncReadPinBox, FileStreamResult, Source, SourceRead}, server::get_server_file_path_array, tools::{auth::{sign_local, ClaimsLocal, ClaimsLocalType}, log::{log_info, LogServiceType}}};

use super::{error::{Error, Result}, users::{ConnectedUser, UserRole}, ModelController};




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
            LibraryRole::Admin  => write!(f, "admin"),
            LibraryRole::Read  => write!(f, "read"),
            LibraryRole::Write  => write!(f, "write"),
            LibraryRole::None  => write!(f, "none"),
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerLibraryInvitation {
	pub code: String,
	pub expires: Option<String>,
	pub library: String,
	pub roles: Vec<LibraryRole>,
    pub limits: LibraryLimits
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
            settings:Some(lib.settings),
            roles: None,

            ..Default::default()
        }
    }
}
impl ServerLibraryForRead {
    fn into_with_role(lib: ServerLibrary, roles: &Vec<LibraryRole>) -> Self {
        ServerLibraryForRead {
            id: lib.id,
            name: lib.name,
            source: Some(lib.source),
            root: lib.root,
            kind: lib.kind,
            crypt: lib.crypt,
            settings:Some(lib.settings),
            roles: Some(roles.to_owned()),
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
}
 
pub(super) fn map_library_for_user(library: ServerLibrary, user: &ConnectedUser) -> Option<ServerLibraryForRead> {
    
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
        },
        ConnectedUser::Anonymous | ConnectedUser::Guest(_) => None,
        ConnectedUser::ServerAdmin => Some(ServerLibraryForRead::from(library)),
        ConnectedUser::Share(claims) => {
            if claims.kind == ClaimsLocalType::Admin {
                Some(ServerLibraryForRead::from(library))
            } else {
                None
            }
        },
        ConnectedUser::UploadKey(key) => {
            let mut library_out = ServerLibraryForRead::into_with_role(library, &vec![LibraryRole::Write]);
            if library_out.id == key.id {
                library_out.root = None;
                library_out.settings = None;
                Some(library_out)
            } else {
                None
            }
                
        },
    }


}



#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct LibrarySocketMessage {
    pub action: ElementAction,
    pub library: ServerLibraryForRead
}

impl LibraryMessage {
    pub fn for_socket(&self, user: &ConnectedUser) -> Option<LibrarySocketMessage> {
        if let Some(library) =  map_library_for_user(self.library.clone(), user) {
            Some(LibrarySocketMessage { action: self.action.clone(), library })
        } else {
            None
        }
    }
}



impl ModelController {
    
	pub async fn get_library(&self, library_id: &str, requesting_user: &ConnectedUser) -> Result<Option<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Read)?;
		let lib = self.store.get_library(library_id).await?;
		if let Some(lib) = lib {
			let return_library = map_library_for_user(lib, &requesting_user);
			Ok(return_library)
		} else {
			Ok(None)
		}
	}

    pub async fn get_internal_library(&self, library_id: &str) -> Result<Option<super::libraries::ServerLibrary>> {
		let lib = self.store.get_library(library_id).await?;
		Ok(lib)
	}

	pub async fn get_libraries(&self, requesting_user: &ConnectedUser) -> Result<Vec<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_role(&UserRole::Read)?;
		let libraries = self.store.get_libraries().await?.into_iter().flat_map(|l|  map_library_for_user(l, &requesting_user));
		Ok(libraries.collect::<Vec<super::libraries::ServerLibraryForRead>>())
	}

    pub async fn get_library_mapped_users(&self, library_id: &str) -> Result<Vec<UserMapping>> {
        let library = self.get_library(library_id, &ConnectedUser::ServerAdmin).await?.ok_or(SourcesError::UnableToFindLibrary(library_id.to_string(), "get_library_mapped_users".to_string()))?;
        
        return Ok(library.settings.and_then(|s| s.map_progress).unwrap_or_default())

	}

    /// Get list of user id this user is currently mapped from
    /// Exemple if user A is mapped to B then passing user A will return B  
    pub async fn get_library_progress_user_mappings(&self, library_id: &str, user_id: String) -> Result<Vec<String>> {
        let mut mappings = vec![];
        let library = self.get_internal_library(library_id).await?.ok_or(SourcesError::UnableToFindLibrary(library_id.to_string(), "get_library_progress_user_mappings".to_string()))?;
        if let Some(mapping) = library.settings.map_progress {
            let filtered = mapping.into_iter().filter(|m| m.from == user_id);
            for mapping in filtered {
                mappings.push(mapping.to);
            }
        }
        return Ok(mappings)
	}

    /// Get list of user id this user is currently mapped to
    /// Exemple if user A is mapped to B then passing user B will return A  
    pub async fn get_library_progress_user_mapped(&self, library_id: &str, user_id: String) -> Result<Vec<String>> {
        let mut mappings = vec![];
        let library = self.get_internal_library(library_id).await?.ok_or(SourcesError::UnableToFindLibrary(library_id.to_string(), "get_library_mapped_users".to_string()))?;
        if let Some(mapping) = library.settings.map_progress {
            let filtered = mapping.into_iter().filter(|m| m.to == user_id);
            for mapping in filtered {
                mappings.push(mapping.from);
            }
        }
        return Ok(mappings)
	}

    /// Get list of users that are either in a to or a from mapping with the *user_id* including *user_id*
    /// plux all users mapped to those users
    pub async fn get_library_progress_merged_users(&self, library_id: &str, user_id: String) -> Result<HashSet<String>> {
        let mut mappings = HashSet::new();
        let library = self.get_internal_library(library_id).await?.ok_or(SourcesError::UnableToFindLibrary(library_id.to_string(), "get_library_mapped_users".to_string()))?;
        if let Some(mapping) = library.settings.map_progress {
            let filtered = mapping.iter().filter(|m| m.to == user_id || m.from == user_id);
            for mapped in filtered {
                mappings.insert(mapped.from.clone());
                mappings.insert(mapped.to.clone());
                let subfil = mapping.iter().filter(|m| &m.to == &mapped.to || &m.from == &mapped.from || &m.to == &mapped.from || &m.from == &mapped.to);
                for m in subfil {
                    mappings.insert(m.from.clone());
                    mappings.insert(m.to.clone());
                }
            }
        }
        return Ok(mappings)
	}

    pub async fn get_library_mapped_user(&self, library_id: &str, mut user_id: String) -> Result<String> {
        let library = self.get_internal_library(library_id).await?.ok_or(SourcesError::UnableToFindLibrary(library_id.to_string(), "get_library_mapped_users".to_string()))?;
        if let Some(mapping) = library.settings.map_progress {
            if let Some(mapping) = mapping.into_iter().find(|m| m.from == user_id) {
                user_id = mapping.to;
            }
        }
        return Ok(user_id)
	}
    /// If library_id is None, return user_id unchanged
    /// If library_id is Some, return mapped user if any, else user_id unchanged
    pub async fn get_optional_library_mapped_user(&self, library_id: Option<&str>, mut user_id: String) -> Result<String> {
        if let Some(library_id) = library_id {
            let library = self.get_internal_library(library_id).await?.ok_or(SourcesError::UnableToFindLibrary(library_id.to_string(), "get_library_mapped_users".to_string()))?;
            if let Some(mapping) = library.settings.map_progress {
                if let Some(mapping) = mapping.into_iter().find(|m| m.from == user_id) {
                    user_id = mapping.to;
                }
            }
            return Ok(user_id)
        }
        Ok(user_id)
	}
 
	pub async fn update_library(&self, library_id: &str, update: ServerLibraryForUpdate, requesting_user: &ConnectedUser) -> Result<Option<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
		self.store.update_library(library_id, update).await?;
        let library = self.store.get_library(library_id).await?;
        if let Some(library) = library { 
            self.cache_update_library(library.clone()).await;
            self.send_library(LibraryMessage { action: crate::domain::ElementAction::Updated, library: library.clone() });
            Ok(map_library_for_user(library, &requesting_user))
        } else {
            Ok(None)
        }
	}

	pub async fn add_library(&self, library_for_add: ServerLibraryForAdd, importData: Option<Vec<u8>>, requesting_user: &ConnectedUser) -> RsResult<Option<super::libraries::ServerLibraryForRead>> {
        
        requesting_user.check_role(&UserRole::Admin)?;
		let library_id = nanoid!();
        let library = ServerLibrary {
                id: library_id.clone(),
                name: library_for_add.name,
                source: library_for_add.source,
                root: library_for_add.root,
                kind: library_for_add.kind,
                crypt: library_for_add.crypt,
                settings: library_for_add.settings,
                plugin: library_for_add.plugin,
                credentials: library_for_add.credentials,

                ..Default::default()
            };
        self.store.add_library(library).await?;
        let user_id = requesting_user.user_id()?;
        self.store.add_library_rights(library_id.clone(), user_id, vec![LibraryRole::Admin], LibraryLimits::default()).await?;
        let library = self.store.get_library(&library_id).await?.ok_or(crate::Error::Error(format!("unable to load librarary from database after creation")))?;

        if let Some(importData) = importData {
            let server_db_path = get_server_file_path_array(vec![&"dbs", &format!("db-{}.db", &library.id)]).await.map_err(|_| Error::CannotOpenDatabase)?;  
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
        
        

        
        log_info(LogServiceType::LibraryCreation, format!("Will do first init of library {}", library.name));
        self.cache_update_library(library.clone()).await;

        let source = self.source_for_library(&library.id).await.map_err(|e| Error::ServiceError("Unable to get library source after init".to_string(), Some(e.to_string())))?;
        let inited = source.init().await;
        if let Err(err) = inited {
            return Err(Error::ServiceError("Unable to init library source".to_string(), Some(err.to_string())).into());
        }

        self.store.add_library_to_store(&library_id).await.map_err(|e| Error::ServiceError("Unable to add library to store".to_string(), Some(e.to_string())))?;
        self.send_library(LibraryMessage { action: crate::domain::ElementAction::Added, library: library.clone() });
        Ok(Some(ServerLibraryForRead::from(library)))

	}
    
	pub async fn remove_library(&self, library_id: &str, requesting_user: &ConnectedUser) -> RsResult<ServerLibraryForRead> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
        let library = self.store.get_library(&library_id).await?.ok_or(SourcesError::UnableToFindLibrary(library_id.to_string(), "get_library_mapped_users".to_string()))?;

        self.cache_remove_library(&library.id).await;
        self.store.remove_library(library_id.to_string()).await?;
        self.send_library(LibraryMessage { action: crate::domain::ElementAction::Deleted, library: library.clone() });
        Ok(ServerLibraryForRead::from(library))
	}

    pub async fn clean_library(&self, library_id: &str, requesting_user: &ConnectedUser) -> crate::error::Result<Vec<(String, u64)>> {
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

    pub async fn add_library_invitation(&self, library_id: &str, roles: Vec<LibraryRole>, limits: LibraryLimits, requesting_user: &ConnectedUser) -> Result<super::libraries::ServerLibraryInvitation> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let invitation = ServerLibraryInvitation {
            code: nanoid!(),
            expires: None,
            library: library_id.to_string(),
            roles,
            limits
        };
        self.store.add_library_invitation(invitation.clone()).await?;
        Ok(invitation)
	}

    

    pub async fn get_watermarks(&self, library_id: &str, requesting_user: &ConnectedUser) -> Result<Vec<String>> {
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

    pub async fn get_watermark(&self, library_id: &str, watermark: &str, requesting_user: &ConnectedUser) -> RsResult<SourceRead> {
        requesting_user.check_library_role(&library_id, LibraryRole::Read)?;

        let watermark = if watermark == "default" {
            "".to_owned()
        } else {
            format!(".{}",watermark)
        };
        let local = self.library_source_for_library(library_id).await?;
        let sourceread = local.get_file(&format!(".watermark{}.png", watermark), None).await?;
            
		Ok(sourceread)
	}


    pub async  fn get_request_share_token(&self, library_id: &str, request: &RsRequest, delay_in_seconds: u64, requesting_user: &ConnectedUser) -> Result<String> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let exp = ClaimsLocal::generate_seconds(delay_in_seconds);
        let claims = ClaimsLocal {
            cr: "service::share_request".to_string(),
            kind: crate::tools::auth::ClaimsLocalType::RequestUrl(request.url.to_string()),
            exp,
        };
        let token = sign_local(claims).await.map_err(|_| Error::UnableToSignShareToken)?;
        Ok(token)
    }

    
}


impl ModelController {


    pub async fn url_to_source(&self, library_id: &str, url: String, requesting_user: &ConnectedUser) -> RsResult<SourceRead> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;

		let request = RsRequest {
			url,
			..Default::default()
		};
		let source = SourceRead::Request(request);

       
		Ok(source)
	}

    pub async fn url_to_reader(&self, library_id: &str, url: String, requesting_user: &ConnectedUser) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let source = self.url_to_source(library_id, url, requesting_user).await?;
		
		let mut reader = source.into_reader(Some(library_id), None, None, Some((self.clone(), requesting_user)), None).await?;
       
		Ok(reader)
	}

        
	pub async fn url_to_bufer(&self, library_id: &str, url: String, requesting_user: &ConnectedUser) -> RsResult<Vec<u8>> {
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
}