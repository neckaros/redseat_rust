


use axum::{body::Body, response::{IntoResponse, Response}};
use futures::TryFutureExt;
use hex_literal::hex;
use http::{header, HeaderMap};
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncReadExt;


use crate::{domain::{backup::{Backup, BackupFile, BackupStatusMessage}, library::LibraryRole, media::{Media, MediaForUpdate}}, error::{RsError, RsResult}, tools::{clock::now, encryption::{ceil_to_multiple_of_16, derive_key, estimated_encrypted_size, random_iv, AesTokioDecryptStream, AesTokioEncryptStream}}};

use super::{error::{Error, Result}, medias::{MediaFileQuery, MediaQuery}, store::sql::backups::BackupInfos, users::{ConnectedUser, UserRole}, ModelController};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupForAdd {
	pub source: String,
    pub credentials: Option<String>,
    pub plugin: Option<String>,
    pub library: String,
    pub path: String,
    pub schedule: Option<String>,
    pub filter: Option<MediaQuery>,
    pub last: Option<i64>,
    pub password: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct BackupForUpdate {
	pub source: Option<String>,
	pub credentials: Option<String>,
    pub plugin: Option<String>,
	pub library: Option<String>,
    pub path: Option<String>,
    pub schedule: Option<String>,
    pub filter: Option<MediaQuery>,
    pub last: Option<i64>,
    pub password: Option<String>,
    pub size: Option<u64>,
}



impl ModelController {

    pub fn send_backup_status(&self, message: BackupStatusMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Admin);
			if r.is_ok() {
				let _ = socket.emit("backup-status", message);
			}
		});
	}



	pub async fn get_backups(&self, requesting_user: &ConnectedUser) -> Result<Vec<Backup>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let credentials = self.store.get_backups().await?;
		Ok(credentials)
	}


    pub async fn get_backup(&self, backup_id: String, requesting_user: &ConnectedUser) -> Result<Option<Backup>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let credential = self.store.get_backup(&backup_id).await?;
		Ok(credential)
	}
    pub async fn get_backup_files_infos(&self, backup_id: &str, requesting_user: &ConnectedUser) -> Result<BackupInfos> {
        requesting_user.check_role(&UserRole::Admin)?;
		let infos = self.store.get_library_backup_files_infos(backup_id).await?;
		Ok(infos)
	}

    pub async fn update_backup(&self, backup_id: &str, update: BackupForUpdate, requesting_user: &ConnectedUser) -> Result<Backup> {
        requesting_user.check_role(&UserRole::Admin)?;
		self.store.update_backup(backup_id, update).await?;
        let backup = self.store.get_backup(backup_id).await?;
        if let Some(backup) = backup { 
            Ok(backup)
        } else {
            Err(Error::NotFound)
        }
	}


    pub async fn add_backup(&self, backup: BackupForAdd, requesting_user: &ConnectedUser) -> Result<Backup> {
        requesting_user.check_role(&UserRole::Admin)?;
        let backup = Backup {
            id: nanoid!(),
            source: backup.source,
            credentials: backup.credentials,
            library: backup.library,
            path: backup.path,
            schedule: backup.schedule,
            filter: backup.filter,
            last: backup.last,
            password: backup.password,
            size: 0,
            plugin: backup.plugin
        };
		self.store.add_backup(backup.clone()).await?;
		Ok(backup)
	}


    pub async fn remove_backup(&self, backup_id: &str, requesting_user: &ConnectedUser) -> Result<Backup> {
        requesting_user.check_role(&UserRole::Admin)?;
        let credential = self.store.get_backup(&backup_id).await?;
        if let Some(credential) = credential { 
            self.store.remove_backup(backup_id.to_string()).await?;
            Ok(credential)
        } else {
            Err(Error::NotFound)
        }
	}

    pub async fn get_library_backup_files(&self, library_id: &str, requesting_user: &ConnectedUser) -> Result<Vec<BackupFile>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
		let backup_files = self.store.get_library_backup_files(&library_id).await?;
		Ok(backup_files)
	}

    pub async fn get_backup_files(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> Result<Vec<BackupFile>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
		let backup_files = self.store.get_backup_files(&library_id, &media_id).await?;
		Ok(backup_files)
	}

    pub async fn check_existing_backup_file(&self, library_id: &str, media_id: &str, source_hash: &str, requesting_user: &ConnectedUser) -> Result<bool> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
		let backup_files = self.store.get_backup_files(&library_id, &media_id).await?;

        Ok(backup_files.into_iter().any(|b| b.sourcehash == source_hash))
	}

    pub async fn get_backup_file(&self, backup_file_id: &str, requesting_user: &ConnectedUser) -> RsResult<BackupFile> {
    
        let backup = self.store.get_backup_file(backup_file_id).await?.ok_or(RsError::BackupFileNotFound(backup_file_id.to_owned()))?;

        Ok(backup)

    }

    pub async fn get_backup_media(&self, library_id: &str, media_id: &str, backup_file_id: Option<&str>, requesting_user: &ConnectedUser) -> RsResult<Response> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;

        let backup_file = if let Some(backup_file_id) = backup_file_id {
            self.get_backup_file(backup_file_id, requesting_user).await?
        } else { 
            let backup_files = self.get_backup_files(library_id, media_id, requesting_user).await?;
            backup_files.first().ok_or(RsError::BackupNotFound(library_id.to_string(), media_id.to_string())).cloned()?
        };

        let backup_info = self.get_backup(backup_file.backup.to_string(), requesting_user).await?.ok_or(RsError::BackupProcessNotFound(backup_file.backup.to_string()))?;

        let source = self.plugin_manager.source_for_backup(backup_info.clone(), self.clone()).await?;

        let mut source_read = source.get_file(&backup_file.path, None).await?;

        let mut reader =  source_read.into_reader(library_id, None, None, Some((self.clone(), requesting_user)), None).await?;

        let key = derive_key(backup_info.password.clone().unwrap_or_default());

        let (asyncwriter, asyncreader) = tokio::io::duplex(256 * 1024);
        let mut streamreader = tokio_util::io::ReaderStream::new(asyncreader);

        let mut decrypt_stream = AesTokioDecryptStream::new(asyncwriter, &key, None)?;

        let source = tokio::spawn(async move {

            
            let mut buffer = vec![0; 1024];
            loop {
                let bytes_read = reader.stream.read(&mut buffer).await.unwrap();
                if bytes_read == 0 {
                    break;
                }
                decrypt_stream.write_decrypted(&buffer[..bytes_read]).await.unwrap();
            }
        
            decrypt_stream.finalize().await.unwrap();
            

           
        }).map_err(|r| RsError::Error("Unable to get plugin writer".to_string()));

        let media = self.get_media(library_id, media_id.to_string(), requesting_user).await?;

        let body = Body::from_stream(streamreader);
        let mut headers = HeaderMap::new();
        if let Some(media) = media {
            headers.insert(header::CONTENT_TYPE, media.mimetype.parse()?);
            headers.insert(header::CONTENT_DISPOSITION, format!("attachment; filename={:?}", media.name).parse()?);
            if let Some(size) = media.size {
                headers.insert(header::CONTENT_LENGTH, size.to_string().parse()?);
            }
        }

        let status =  axum::http::StatusCode::OK;
        Ok((status, headers, body).into_response())

		
	}

    pub async fn add_backup_file(&self, backup: BackupFile, requesting_user: &ConnectedUser) -> Result<BackupFile> {
        requesting_user.check_role(&UserRole::Admin)?;
        
		self.store.add_backup_file(backup.clone()).await?;
		Ok(backup)
	}
    

    pub async fn upload_backup_media(&self, backup_id: &str, media_id: &str, id: Option<String>, requesting_user: &ConnectedUser) -> RsResult<BackupFile> {
        let backup_info = self.get_backup(backup_id.to_string(), requesting_user).await?.ok_or(RsError::BackupProcessNotFound(backup_id.to_string()))?;
        let media_info = self.get_media(&backup_info.library, media_id.to_string(), requesting_user).await?.ok_or(RsError::NotFound)?;
        let media_info_string = serde_json::to_string(&media_info)?;
        let media_info_string_len = ceil_to_multiple_of_16(media_info_string.len());

        requesting_user.check_library_role(&backup_info.library, LibraryRole::Admin)?;

        let source = self.plugin_manager.source_for_backup(backup_info.clone(), self.clone()).await?;

        let mut source_read = self.library_file(&backup_info.library, media_id, None, MediaFileQuery { raw: true, ..Default::default()}, requesting_user).await?;
        let mut reader = source_read.into_reader(&backup_info.library, None, None, Some((self.clone(), requesting_user)), None).await?;

        let key = derive_key(backup_info.password.clone().unwrap_or_default());
        let iv = random_iv();
        let id = id.unwrap_or(nanoid!());
        let encrypted_size = estimated_encrypted_size(media_info.size.unwrap_or(0), 0, media_info_string.len() as u64);
        
        let writer = source.writer(&id, Some(encrypted_size), Some(media_info.mimetype.to_owned())).await?;
        let mut decrypt_stream = AesTokioEncryptStream::new(writer.1, &key, &iv, Some(media_info.mimetype.to_owned()), None, Some(media_info_string))?;
        let read_process = tokio::spawn(async move {
            
            let mut buffer = vec![0; 1024];
            loop {
                let bytes_read = reader.stream.read(&mut buffer).await?;
                if bytes_read == 0 {
                    break;
                }
                decrypt_stream.write_encrypted(&buffer[..bytes_read]).await?;
            }
        
            decrypt_stream.finalize().await?;
            
            Ok::<_, RsError>(())
           
        }).map_err(|r| RsError::Error("Unable to spawn encryption process".to_string()));

        read_process.await??; 
        let new_source = writer.0.await??;
        let mut infos = MediaForUpdate::default();

        source.fill_infos(&new_source, &mut infos).await?;
       let backup = BackupFile { id, backup: backup_info.id, library: backup_info.library, file: media_info.id.to_owned(), path: new_source, hash: infos.md5.clone().unwrap_or_default(), sourcehash: media_info.md5.clone().unwrap_or_default(), size: infos.size.unwrap_or_default(), date: media_info.max_date(), iv: Some(hex::encode(&iv)), thumb_size: None, info_size: Some(media_info_string_len as u64), error: None };
       Ok(backup)

		
	}


    pub async fn remove_backup_file(&self, backup_file_id: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
        let backup_file = self.get_backup_file(backup_file_id, requesting_user).await?;
        let backup_info = self.get_backup(backup_file.backup.to_string(), requesting_user).await?.ok_or(RsError::BackupProcessNotFound(backup_file.backup.to_string()))?;
        requesting_user.check_library_role(&backup_info.library, LibraryRole::Admin)?;
        
        let source = self.plugin_manager.source_for_backup(backup_info.clone(), self.clone()).await?;
        
        source.remove(&backup_file.path).await?;
        Ok(())
	}

    pub async fn remove_backup_files_for_media(&self, backup_id: &str, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> RsResult<usize> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let backup_files = self.get_backup_files(library_id, media_id, requesting_user).await?;
        let mut total_deleted = 0usize;
        for backup_file in backup_files.into_iter().filter(|b| b.backup == backup_id) {
            self.remove_backup_file(&backup_file.id, requesting_user).await?;
            total_deleted += 1;
        }
        Ok(total_deleted)
	}
}
