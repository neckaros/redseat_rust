


use std::{io::Cursor, path::PathBuf};

use axum::{body::Body, response::{IntoResponse, Response}};
use futures::{future::ok, TryFutureExt};
use hex_literal::hex;
use http::{header, request, HeaderMap};
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha256::try_async_digest;
use tokio::{fs::{self, File}, io::{copy, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader}, sync::mpsc};
use tokio_stream::StreamExt;
use tokio_util::io::{ReaderStream, StreamReader};


use crate::{domain::{backup::{self, Backup, BackupError, BackupFile, BackupFileProgress, BackupMessage, BackupProcessStatus, BackupStatus, BackupWithStatus}, library::{LibraryRole, ServerLibrary}, media::{self, Media, MediaForUpdate, DEFAULT_MIME}, progress::{RsProgress, RsProgressType}}, error::{RsError, RsResult}, model::libraries::ServerLibraryForAdd, plugins::sources::{async_reader_progress::ProgressReader, error::SourcesError, AsyncReadPinBox, FileStreamResult, SourceRead}, routes::mw_range::RangeDefinition, tools::{clock::now, encryption::{ceil_to_multiple_of_16, derive_key, estimated_encrypted_size, random_iv, AesTokioDecryptStream, AesTokioEncryptStream}, log::{log_error, log_info}}};

use super::{error::{Error, Result}, medias::{MediaFileQuery, MediaQuery, MediaSource}, store::sql::backups::BackupInfos, users::{ConnectedUser, UserRole}, ModelController};
use crate::routes::sse::SseEvent;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupForAdd {
	pub name: String,
	pub source: String,
    pub credentials: Option<String>,
    pub plugin: Option<String>,
    pub library: Option<String>,
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
    pub name: Option<String>,
}



impl ModelController {

    pub fn send_backup_status(&self, message: BackupMessage) {
		self.broadcast_sse(SseEvent::Backups(message));
	}

    pub fn send_backup_file_status(&self, message: BackupFileProgress) {
		self.broadcast_sse(SseEvent::BackupsFiles(message));
	}

    pub async fn set_backup_status(&self, status: BackupProcessStatus) -> RsResult<()> {
        let backup_id = status.backup.clone();
		let mut progresses = self.backup_processes.write().await;
        if let Some(index) = progresses.iter().position(|b| b.backup == status.backup) {
            progresses.remove(index);
        }
        progresses.push(status.clone());

        let backup = self.get_backup(&backup_id, &ConnectedUser::ServerAdmin).await?;
        if let Some(backup) = backup {
            let backup_with_status = BackupWithStatus {
                backup,
                status: Some(status)
            };
            let message = BackupMessage {
                action: crate::domain::ElementAction::Updated,
                backup: backup_with_status,
            };
            self.send_backup_status(message);
        }
        Ok(())
	}

    pub async fn is_processing_backup(&self, backup_id: &str) -> bool {
		self.backup_processes.read().await.iter().any(|b| b.backup == backup_id && b.status == BackupStatus::InProgress)
	}
    pub async fn to_backups_with_status(&self, backups: Vec<Backup>) -> Vec<BackupWithStatus> {
        let mut backups_with_status = vec![];
		for backup in backups {
            let backup_id = backup.id.to_owned();
            let backup_with_status = BackupWithStatus {
                backup: backup,
                status: self.backup_processes.read().await.iter().find(|b| b.backup == backup_id).cloned(),
            };
            backups_with_status.push(backup_with_status);
        }
        backups_with_status
	}

    pub async fn to_backup_with_status(&self, backup: Backup) -> BackupWithStatus {
        let backup_id = backup.id.to_owned();
        let backup_with_status = BackupWithStatus {
            backup: backup,
            status: self.backup_processes.read().await.iter().find(|b| b.backup == backup_id).cloned(),
        };
        backup_with_status
	}




	pub async fn get_backups(&self, requesting_user: &ConnectedUser) -> Result<Vec<Backup>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let backups = self.store.get_backups().await?;
		Ok(backups)
	}
	pub async fn get_backups_with_status(&self, requesting_user: &ConnectedUser) -> Result<Vec<BackupWithStatus>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let backups: Vec<Backup> = self.get_backups(requesting_user).await?;
        let backups_with_status = self.to_backups_with_status(backups).await;
		Ok(backups_with_status)
	}

    pub async fn get_backup(&self, backup_id: &str, requesting_user: &ConnectedUser) -> Result<Option<Backup>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let credential = self.store.get_backup(&backup_id).await?;
		Ok(credential)
	}
    pub async fn get_backup_with_status(&self, backup_id: &str, requesting_user: &ConnectedUser) -> Result<Option<BackupWithStatus>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let backup= if let Some(backup) = self.get_backup(backup_id, requesting_user).await? {
            Some(self.to_backup_with_status(backup).await)
        } else {
            None
        };
        
		Ok(backup)
	}
    
    pub async fn get_backup_files_infos(&self, backup_id: &str, requesting_user: &ConnectedUser) -> Result<BackupInfos> {
        requesting_user.check_role(&UserRole::Admin)?;
		let infos = self.store.get_backup_files_infos(backup_id).await?;
		Ok(infos)
	}

    pub async fn update_backup(&self, backup_id: &str, update: BackupForUpdate, requesting_user: &ConnectedUser) -> Result<Backup> {
        requesting_user.check_role(&UserRole::Admin)?;
		self.store.update_backup(backup_id, update).await?;
        let backup = self.store.get_backup(backup_id).await?.ok_or(SourcesError::UnableToFindBackup(backup_id.to_string(), "update_backup".to_string()))?;

        let backup_with_status = BackupWithStatus { backup: backup.clone(), status: None};
        let message = BackupMessage { action: crate::domain::ElementAction::Updated, backup: backup_with_status };
        self.send_backup_status(message);
        Ok(backup)

	}


    pub async fn add_backup(&self, backup: BackupForAdd, requesting_user: &ConnectedUser) -> Result<Backup> {
        requesting_user.check_role(&UserRole::Admin)?;
        let backup = Backup {
            id: nanoid!(),
            name: backup.name,
            source: backup.source,
            credentials: backup.credentials,
            library: backup.library,
            path: backup.path,
            schedule: backup.schedule,
            filter: backup.filter,
            last: backup.last,
            password: backup.password,
            size: 0,
            plugin: backup.plugin,
        };
		self.store.add_backup(backup.clone()).await?;
		Ok(backup)
	}


    pub async fn remove_backup(&self, backup_id: &str, requesting_user: &ConnectedUser) -> Result<Backup> {
        requesting_user.check_role(&UserRole::Admin)?;
        let credential = self.store.get_backup(&backup_id).await?.ok_or(SourcesError::UnableToFindBackup(backup_id.to_string(), "remove_backup".to_string()))?;
     
        self.store.remove_backup(backup_id.to_string()).await?;
        Ok(credential)
	}


    /// Get all backup files for a backup
    /// 
    /// UNPROTECTED INTERNAL USE ONLY
    pub async fn get_backup_backup_files(&self, backup_id: &str) -> Result<Vec<BackupFile>> {
        let backup_files = self.store.get_backup_backup_files(&backup_id).await?;
        Ok(backup_files)
    }

    /// Get all the backup files for a library, whatever the backup
    pub async fn get_library_backup_files(&self, library_id: &str, requesting_user: &ConnectedUser) -> Result<Vec<BackupFile>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
		let backup_files = self.store.get_library_backup_files(&library_id).await?;
		Ok(backup_files)
	}

    

    /// For a specific media id get all the files for a specific backup
    /// 
    /// UNPROTECTED INTERNAL USE ONLY
    pub async fn get_backup_media_backup_files(&self, backup_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> Result<Vec<BackupFile>> {
		let backup_files = self.store.get_backup_media_backup_files(&backup_id, &media_id).await?;
		Ok(backup_files)
	}


    /// For a specific media id get all the files for a specific library whatever the backup
    pub async fn get_library_media_backup_files(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> Result<Vec<BackupFile>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
        let backup_files = self.store.get_library_media_backup_files(&library_id, &media_id).await?;
        Ok(backup_files)
    }


    /// Check if a media with the same hash already exist for this backup
    /// 
    /// UNPROTECTED INTERNAL USE ONLY
    pub async fn check_backup_existing_media(&self, backup_id: &str, media_id: &str, source_hash: &str) -> Result<bool> {
		let backup_files = self.store.get_backup_media_backup_files(&backup_id, &media_id).await?;

        Ok(backup_files.into_iter().any(|b| b.sourcehash == source_hash))
	}

    /// Get a specific backup file from ID
    /// 
    /// UNPROTECTED INTERNAL USE ONLY
    pub async fn get_backup_file(&self, backup_file_id: &str) -> RsResult<BackupFile> {
    
        let backup = self.store.get_backup_file(backup_file_id).await?.ok_or(RsError::BackupFileNotFound(backup_file_id.to_owned()))?;

        Ok(backup)

    }

    /// Get the SourceRead for a backup file
    /// 
    /// If no `backup_file_id` is provided, the last backup will be fetched
    pub async fn get_backup_media(&self, library_id: &str, media_id: &str, backup_file_id: Option<&str>, requesting_user: &ConnectedUser) -> RsResult<SourceRead> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;

        let backup_file = if let Some(backup_file_id) = backup_file_id {
            self.get_backup_file(backup_file_id).await?
        } else { 
            let backup_files = self.get_library_media_backup_files(library_id, media_id, requesting_user).await?;
            backup_files.first().ok_or(RsError::BackupNotFound(library_id.to_string(), media_id.to_string())).cloned()?
        };

        let backup_info = self.get_backup(&backup_file.backup, requesting_user).await?.ok_or(RsError::BackupProcessNotFound(backup_file.backup.to_string()))?;

        let source = self.plugin_manager.provider_for_backup(backup_info.clone(), self.clone()).await?;
        let mut source_read = source.get_file(&backup_file.path, None).await?;

       
        let source = if let Some(password) = backup_info.password.clone() {

            let mut reader =  source_read.into_reader(Some(&library_id.to_string()), None, None, Some((self.clone(), requesting_user)), None).await?;

            let media = self.get_media(library_id, media_id.to_string(), requesting_user).await?;


            let key = derive_key(backup_info.password.clone().unwrap_or_default());

            let (asyncwriter, asyncreader) = tokio::io::duplex(256 * 1024);
            //let mut streamreader = tokio_util::io::ReaderStream::new(asyncreader);

            let mut decrypt_stream = AesTokioDecryptStream::new(asyncwriter, &key, None)?;

            tokio::spawn(async move {


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

            SourceRead::Stream(FileStreamResult {
                stream: Box::pin(asyncreader),
                size: media.as_ref().map(|m| m.item.size.to_owned()).flatten(),
                accept_range: false,
                range: None,
                mime: media.as_ref().map(|m| m.item.mimetype.to_owned()),
                name: media.as_ref().map(|m| m.item.name.to_owned()),
                cleanup: None,
            })
        } else {
            source_read
        };

        Ok(source)
	}


    pub async fn check_role_for_backup_id(&self, backup_file_id: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
        let backup_file = self.get_backup_file(backup_file_id).await?;
        ModelController::check_role_for_backup(&backup_file, requesting_user)
    }

    pub fn check_role_for_backup(backup_file: &BackupFile, requesting_user: &ConnectedUser) -> RsResult<()> {
        if let Some(library_id) = &backup_file.library {
            requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
            Ok(())
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
            Ok(())
        }
        
    }

    /// Get the SourceRead for a backup file
    /// 
    /// If no `backup_file_id` is provided, the last backup will be fetched
    pub async fn get_backup_file_reader(&self, backup_file_id: &str, requesting_user: &ConnectedUser) -> RsResult<SourceRead> {
        let backup_file = self.get_backup_file(backup_file_id).await?;

        ModelController::check_role_for_backup(&backup_file, requesting_user)?;



        let backup_info = self.get_backup(&backup_file.backup, requesting_user).await?.ok_or(RsError::BackupProcessNotFound(backup_file.backup.to_string()))?;

        let source = self.plugin_manager.provider_for_backup(backup_info.clone(), self.clone()).await?;
        let mut source_read = source.get_file(&backup_file.path, None).await?;

       
        let source = if let Some(password) = backup_info.password.clone() {

            let mut reader =  source_read.into_reader(backup_file.library.as_deref(), None, None, Some((self.clone(), requesting_user)), None).await?;

            let media = if let Some(library_id) = backup_file.library {
                self.get_media(&library_id, backup_file.file.clone(), requesting_user).await?
            } else {
                None
            };


            let key = derive_key(backup_info.password.clone().unwrap_or_default());

            let (asyncwriter, asyncreader) = tokio::io::duplex(256 * 1024);
            //let mut streamreader = tokio_util::io::ReaderStream::new(asyncreader);

            let mut decrypt_stream = AesTokioDecryptStream::new(asyncwriter, &key, None)?;

            tokio::spawn(async move {


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

            SourceRead::Stream(FileStreamResult {
                stream: Box::pin(asyncreader),
                size: media.as_ref().map(|m| m.item.size.to_owned()).flatten(),
                accept_range: false,
                range: None,
                mime: media.as_ref().map(|m| m.item.mimetype.to_owned()),
                name: media.as_ref().map(|m| m.item.name.to_owned()),
                cleanup: None,
            })
        } else {
            source_read
        };

        Ok(source)
	}

    pub async fn add_backup_file(&self, backup: BackupFile, requesting_user: &ConnectedUser) -> Result<BackupFile> {
        requesting_user.check_role(&UserRole::Admin)?;
        
		self.store.add_backup_file(backup.clone()).await?;
		Ok(backup)
	}
    
    /// UNPROTECTED INTERNAL USAGE ONLY
    pub async fn upload_backup(&self, source_read: SourceRead, file_id: String, backup_id: String, sourcehash: String, modified: i64, library_id: Option<String>, infos: Option<Media>, id: Option<String>) -> RsResult<BackupFile> {
        
        let id = id.unwrap_or(nanoid!());

        let backup_info = self.get_backup(&backup_id, &ConnectedUser::ServerAdmin).await?.ok_or(RsError::BackupProcessNotFound(backup_id.to_string()))?;

        let media_info_string = if let Some(infos) = infos.as_ref() { Some(serde_json::to_string(infos)?) } else { None };
        let media_info_string_len =  media_info_string.as_ref().map(|i| i.len() as u64);
        let media_info_string_enc_len =  media_info_string_len.map(|l| ceil_to_multiple_of_16(l as usize) as u64);

        let initial_message = BackupFileProgress::new(file_id.clone(), backup_id.clone(), library_id.clone(), source_read.filename(), source_read.size(), id.to_string(), BackupStatus::InProgress, 0, None);
        self.send_backup_file_status(initial_message.clone());
        let tx_progress = self.create_backup_progress_sender(initial_message.clone());

        let provider = self.plugin_manager.provider_for_backup(backup_info.clone(), self.clone()).await?;

        let source_size = source_read.size();
        let source_mime = source_read.mimetype();
        let source_name = source_read.filename();
        let mut reader = source_read.into_reader(backup_info.library.as_deref(), None, None, Some((self.clone(), &ConnectedUser::ServerAdmin)), None).await?;

        let mut reader = ProgressReader::new(reader.stream, RsProgress { id: id.clone(), total: source_size, current: Some(0), kind: RsProgressType::Transfert, filename: source_name.clone() }, tx_progress.clone());


        let backup = if let Some(password) = backup_info.password.clone() {
            let key = derive_key(password.clone());
            let iv = random_iv();
            let encrypted_size = if let Some(source_size) = source_size { Some(estimated_encrypted_size(source_size, 0, media_info_string_len.clone().unwrap_or(0))) } else { None };
            let writer = provider.writer(&id, encrypted_size, source_mime.clone()).await?;
            let mut crypt_stream = AesTokioEncryptStream::new(writer.1, &key, &iv, source_mime.clone(), None, media_info_string)?;
    
    
    
            let read_process = tokio::spawn(async move {
                
                let mut buffer = vec![0; 1024];
                loop {
                    let bytes_read = reader.read(&mut buffer).await?;
                    if bytes_read == 0 {
                        break;
                    }
                    crypt_stream.write_encrypted(&buffer[..bytes_read]).await?;
                }
            
                crypt_stream.finalize().await?;
                
                Ok::<_, RsError>(())
               
            }).map_err(|r| RsError::Error("Unable to spawn encryption process".to_string()));
            read_process.await??; 
            let backup_file_source = writer.0.await??;
        

            let mut infos = MediaForUpdate::default();
            provider.fill_infos(&backup_file_source, &mut infos).await?;

            let mut message = initial_message.clone();
            message.status = BackupStatus::Done;
            message.progress = infos.size.clone().unwrap_or(0);
            message.size = infos.size.clone();
            self.send_backup_file_status(message);
            let backup = BackupFile { id, backup: backup_info.id, library: backup_info.library, file: file_id, path: backup_file_source, hash: infos.md5.clone().unwrap_or_default(), sourcehash, size: source_size.unwrap_or_default(), modified, added: now().timestamp_millis(), iv: Some(hex::encode(&iv)), thumb_size: None, info_size: media_info_string_enc_len, error: None };
            backup
        } else {
            let mut writer = provider.writer(&infos.as_ref().map(|i| i.name.clone()).unwrap_or(file_id.clone()), source_size, source_mime.clone()).await?;
            copy(&mut reader, &mut writer.1).await?;
            writer.1.flush().await?;
            writer.1.shutdown().await?;
            let backup_file_source = writer.0.await??;
            drop(reader);
        
            let mut infos = MediaForUpdate::default();
            provider.fill_infos(&backup_file_source, &mut infos).await?;

            let mut message = initial_message.clone();
            message.status = BackupStatus::Done;
            message.progress = infos.size.clone().unwrap_or(0);
            message.size = infos.size.clone();
            self.send_backup_file_status(message);
            let backup = BackupFile { id, backup: backup_info.id, library: backup_info.library.clone(), file: file_id, path: backup_file_source, hash: infos.md5.clone().unwrap_or_default(), sourcehash, size: source_size.unwrap_or_default(), modified, added: now().timestamp_millis(), iv: None, thumb_size: None, info_size: None, error: None };
            backup
        };
         Ok(backup)

		
	}

    pub async fn upload_backup_media(&self, backup_id: &str, library_id: &str, media_id: &str, id: Option<String>, requesting_user: &ConnectedUser) -> RsResult<BackupFile> {
        
        let id = id.unwrap_or(nanoid!());
        let media_info = self.get_media(&library_id, media_id.to_string(), requesting_user).await?.ok_or(SourcesError::UnableToFindMedia(library_id.to_string(), media_id.to_string(), "upload_backup_media".to_string()))?.item;
        let mut source_read = self.library_file(&library_id, media_id, None, MediaFileQuery { raw: true, ..Default::default()}, requesting_user).await?;

        let backup_file = self.upload_backup(source_read, media_id.to_string(), backup_id.to_string(), media_info.md5.clone().unwrap_or("none".to_string()), media_info.max_date(),Some(library_id.to_string()), Some(media_info), Some(id)).await?;

        Ok(backup_file)


		
	}

    pub async fn upload_backup_path(&self, backup_info: Backup, file_id: &str, path: PathBuf, name: String, library: Option<ServerLibrary>) -> RsResult<BackupFile> {
        let sourcehash = try_async_digest(&path).await?;
        let id = nanoid!();
        let existing_db_bakcups = self.get_backup_media_backup_files(&backup_info.id, file_id, &ConnectedUser::ServerAdmin).await?;

        let exist = existing_db_bakcups.into_iter().find(|b| b.sourcehash == sourcehash);
        
        if let Some(exist) = exist {
            log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backup {} identical already uploaded. Ignoring", file_id));
            Ok(exist)
        } else {
            let file = File::open(path.clone()).await?;
                
            let metadata = file.metadata().await?;
            let mut file_size = metadata.len();

            let file_content = fs::read(path).await?;

            let reader = if let Some(library) = library {
            

                let library_add = ServerLibraryForAdd { name: library.name, source: library.source, root: library.root, settings: library.settings, kind: library.kind, crypt: library.crypt, credentials: library.credentials, plugin: library.plugin  };
                // Create prefix
                let json_str = serde_json::to_string(&library_add)?;
                let json_bytes = json_str.as_bytes();
                let size = json_bytes.len() as i32;
                let size_bytes = size.to_le_bytes();

    
                // Combine everything into a single buffer
                let mut combined_data = Vec::new();
                combined_data.extend_from_slice(&size_bytes);
                combined_data.extend_from_slice(json_bytes);
                combined_data.extend_from_slice(&file_content);
                
                file_size += 4 + json_bytes.len() as u64;
                
                combined_data

            } else {
                file_content
            };

            println!("Backup {} prepared", file_id);
             let async_reader: AsyncReadPinBox = Box::pin(Cursor::new(reader));

            let file_stream = FileStreamResult {
                stream: async_reader,
                size: Some(file_size),
                accept_range: false,
                range: None,
                mime: Some(DEFAULT_MIME.to_string()),
                name: Some(name),
                cleanup: None,
            };
            let source_read = SourceRead::Stream(file_stream);
            println!("Backup {} uploading", file_id);
            let backup_file = self.upload_backup(source_read, file_id.to_string(), backup_info.id.clone(), sourcehash, now().timestamp_millis(), backup_info.library.clone(), None, Some(id)).await?;
            println!("Backup {} uploaded", file_id);
            let backup_file = self.add_backup_file(backup_file, &ConnectedUser::ServerAdmin).await?;
            println!("upload path");
            Ok(backup_file)
        }
		
	}

    pub async fn remove_backup_file(&self, backup_file_id: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
        let backup_file = self.get_backup_file(backup_file_id).await?;
        let backup_info = self.get_backup(&backup_file.backup, requesting_user).await?.ok_or(RsError::BackupProcessNotFound(backup_file.backup.to_string()))?;
        ModelController::check_role_for_backup(&backup_file, requesting_user)?;
        
        let source = self.plugin_manager.provider_for_backup(backup_info.clone(), self.clone()).await?;
        
        source.remove(&backup_file.path).await?;
        self.store.remove_backup_file(backup_file.id.to_owned()).await?;
        Ok(())
	}

    pub async fn remove_backup_storage_file(&self, backup_id: &str, path: &str) -> RsResult<()> {
        let backup_info = self.get_backup(backup_id, &ConnectedUser::ServerAdmin).await?.ok_or(RsError::BackupProcessNotFound(backup_id.to_string()))?;
        let source = self.plugin_manager.provider_for_backup(backup_info.clone(), self.clone()).await?;
        
        source.remove(path).await?;
        Ok(())
	}

    pub async fn remove_backup_files_for_media(&self, backup_id: &str, media_id: &str, older_than: Option<i64>, not_id: Option<String>, requesting_user: &ConnectedUser) -> RsResult<usize> {
        let backup_files = self.get_backup_media_backup_files(backup_id, media_id, requesting_user).await?;
        let mut total_deleted = 0usize;
        for backup_file in backup_files {
            if let Some(older_than) = older_than {
                if backup_file.added > older_than {
                    continue;
                }
            }
            if let Some(not_id) = not_id.as_ref() {
                if &backup_file.id == not_id {
                    continue;
                }
            }
            let r = self.remove_backup_file(&backup_file.id, requesting_user).await;
            match r {
                Ok(_) => total_deleted += 1,
                Err(_) => log_error(crate::tools::log::LogServiceType::Scheduler, format!("Could not delete backup {} of file {}", backup_file.id, backup_file.file)),
            }
            
        }
        Ok(total_deleted)
	}


    pub fn create_backup_progress_sender(&self, template: BackupFileProgress) -> mpsc::Sender<RsProgress> {
        //Progress
        let mc_progress = self.clone();
        let (tx_progress, mut rx_progress) = mpsc::channel::<RsProgress>(100);
        tokio::spawn(async move {
            let mut last_send = 0;
            let mut last_type: Option<RsProgressType> = None;
            
            let start_time = std::time::Instant::now();

            let mut remaining = None;

            while let Some(mut progress) = rx_progress.recv().await {
                let percent = progress.percent();
                if let Some(percent) = percent {
                    if percent > 0.01f32 {
                        let duration_from_start = start_time.elapsed().as_secs_f32();
                        let remaining_time = duration_from_start / percent * (1f32 - percent);
                        //println!("FFMPEG remaining time: {}", remaining_time);
                        
                        remaining = Some(remaining_time as u64);
                    }
                }

                let current = progress.current.unwrap_or(1);
                if progress.current == progress.total || last_send == 0 || current < last_send || current - last_send  > 1000000 || Some(&progress.kind) != last_type.as_ref() {
                    last_type = Some(progress.kind.clone());
                    last_send = current;
                    let mut message = template.clone();
                    message.status = BackupStatus::InProgress;
                    message.progress = progress.current.unwrap_or_default();
                    mc_progress.send_backup_file_status(message);
                }
            }
        });
        tx_progress
    }


    pub async fn add_backup_error(&self, error: BackupError, requesting_user: &ConnectedUser) -> Result<()> {
        requesting_user.check_role(&UserRole::Admin)?;

		self.store.add_backup_error(error).await?;
        Ok(())
	}
}
