use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use super::{
    embeddings::check_library_wide_access,
    error::Error,
    libraries::LibraryEncryptionStatus,
    store::sql::library::{
        embeddings::MediaEmbeddingCounts,
        stats::{LibraryDuplicates, LibraryEntityCounts, LibraryFaceCounts, LibraryMediaStats},
    },
    users::ConnectedUser,
    ModelController,
};
use crate::{
    domain::library::{LibraryRole, LibraryStorage, ServerLibrary},
    error::RsResult,
    plugins::sources::local_provider_for_library,
    server::get_server_file_path_array,
    tools::{
        file_tools::folder_size,
        log::{log_error, LogServiceType},
    },
};

/// A provider that does not answer in time is left out of the stats.
const STORAGE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    pub medias: LibraryMediaStats,
    /// Left out for virtual libraries and when the provider cannot tell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<LibraryStorage>,
    pub entities: LibraryEntityCounts,
    pub duplicates: LibraryDuplicates,
    pub processing: LibraryProcessing,
    /// Left out until a backup of the library has completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup: Option<LibraryLastBackup>,
    pub server: LibraryServerFootprint,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LibraryProcessing {
    pub faces: LibraryFaceCounts,
    pub embeddings: MediaEmbeddingCounts,
    pub encryption: LibraryEncryptionStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryLastBackup {
    pub id: String,
    pub name: String,
    /// When it last completed (ms).
    pub date: i64,
    /// Bytes the backup holds at its destination, every kept version included.
    pub size: u64,
}

/// What Redseat itself stores for the library, in bytes.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LibraryServerFootprint {
    /// Library database, write-ahead log included.
    pub database_size: u64,
    /// Data folder: thumbnails, portraits, faces and caches.
    pub data_size: u64,
}

impl ModelController {
    pub async fn get_library_stats(
        &self,
        library_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<LibraryStats> {
        check_library_wide_access(requesting_user)?;
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let library = self
            .get_internal_library(library_id)
            .await?
            .ok_or(Error::LibraryNotFound(library_id.to_string()))?;
        let store = self.store.get_library_store(library_id)?;

        let (content, storage, server, embeddings) = tokio::join!(
            store.get_content_stats(),
            self.library_storage(library_id),
            library_server_footprint(&library),
            self.media_embedding_counts(library_id),
        );
        let content = content?;
        let encryption = self
            .get_library_encryption_status(library_id, requesting_user)
            .await?;
        let last_backup = self
            .store
            .get_backups()
            .await?
            .into_iter()
            .filter(|backup| backup.library.as_deref() == Some(library_id))
            .filter_map(|backup| {
                Some(LibraryLastBackup {
                    date: backup.last?,
                    id: backup.id,
                    name: backup.name,
                    size: backup.size,
                })
            })
            .max_by_key(|backup| backup.date);

        Ok(LibraryStats {
            medias: content.medias,
            storage,
            entities: content.entities,
            duplicates: content.duplicates,
            processing: LibraryProcessing {
                faces: content.faces,
                embeddings: embeddings?,
                encryption,
            },
            last_backup,
            server: server?,
        })
    }

    /// Asks the provider without the encryption gate: it reads no library file, so it
    /// does not wait for an encryption change to finish.
    async fn library_storage(&self, library_id: &str) -> Option<LibraryStorage> {
        let storage = async {
            self.source_for_library_unchecked(library_id)
                .await?
                .storage()
                .await
        };
        match tokio::time::timeout(STORAGE_TIMEOUT, storage).await {
            Ok(Ok(storage)) => storage,
            Ok(Err(error)) => {
                log_error(
                    LogServiceType::Source,
                    format!("Unable to get the storage of library {library_id}: {error:#}"),
                );
                None
            }
            Err(_) => {
                log_error(
                    LogServiceType::Source,
                    format!("Storage of library {library_id} timed out after {STORAGE_TIMEOUT:?}"),
                );
                None
            }
        }
    }
}

async fn library_server_footprint(library: &ServerLibrary) -> RsResult<LibraryServerFootprint> {
    let database =
        get_server_file_path_array(vec!["dbs", &format!("db-{}.db", library.id)]).await?;
    let data = local_provider_for_library(library)
        .await?
        .root()
        .to_path_buf();
    Ok(tokio::task::spawn_blocking(move || {
        let mut wal = database.clone().into_os_string();
        wal.push("-wal");
        LibraryServerFootprint {
            database_size: [database, PathBuf::from(wal)]
                .iter()
                .filter_map(|path| std::fs::metadata(path).ok())
                .map(|metadata| metadata.len())
                .sum(),
            data_size: folder_size(&data),
        }
    })
    .await?)
}
