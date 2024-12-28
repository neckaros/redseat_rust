use axum::{async_trait, Error};
use chrono::{DateTime, Duration};
use human_bytes::human_bytes;
use nanoid::nanoid;
use rs_plugin_common_interfaces::ElementType;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::domain::backup::{Backup, BackupError, BackupFile, BackupFileProgress, BackupProcessStatus};
use crate::domain::deleted;
use crate::domain::movie::Movie;
use crate::domain::serie::{Serie, SerieStatus};
use crate::error::RsError;
use crate::model::backups::BackupForUpdate;
use crate::model::deleted::DeletedQuery;
use crate::model::episodes::{EpisodeForUpdate, EpisodeQuery};
use crate::model::movies::MovieQuery;
use crate::model::series::SerieForUpdate;
use crate::model::store::sql::library::medias::MediaBackup;
use crate::plugins::sources::path_provider::PathProvider;
use crate::server::get_server_file_path_array;
use crate::tools::clock::{now, Clock};
use crate::{domain::library, error::RsResult, model::{series::SerieQuery, users::ConnectedUser, ModelController}, plugins::sources::Source, tools::{clock::UtcDate, log::{log_error, log_info}}};

use super::RsSchedulerTask;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupTask {
    pub specific_backup: Option<String>
}

impl BackupTask {

}

#[async_trait]
impl RsSchedulerTask for BackupTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        let connected_user = &ConnectedUser::ServerAdmin;
        let backups = mc.get_backups(&connected_user).await?;
        let backups = if let Some(specific_backup) = &self.specific_backup {
            backups.into_iter().filter(|l| &l.id == specific_backup).collect()
        } else {
            backups
        };

        for backup in backups {
            let existing = mc.is_processing_backup(&backup.id).await;
            if existing {
                log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backup {} already procession", backup.id));
                continue;
            }

            if let Some(library_id) = &backup.library {
                let backup_files_infos = mc.get_backup_files_infos(&backup.id, &ConnectedUser::ServerAdmin).await?;
            

                let media_query = backup.filter.clone().unwrap_or_default();
                let backup_medias = mc.get_medias_to_backup(library_id, backup_files_infos.max_date.unwrap_or(i64::min_value()), media_query, &ConnectedUser::ServerAdmin).await?;
                let total = backup_medias.len() as u64;
                let total_size: u64 = backup_medias.iter().filter_map(|backup| backup.size).sum();
                log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backing up {} medias for size of {} from library {}", backup_medias.len(), human_bytes(total_size as f64), library_id));
                //println!("medias backups: {:?}", backup_medias);
                        
                let deleted = mc.get_deleted(library_id, DeletedQuery { after: backup.last, kind: Some(ElementType::Media), ..Default::default() }, &ConnectedUser::ServerAdmin).await?; 
                let backed_up = mc.get_backup_backup_files(&backup.id).await?;
                


                for delete in deleted {
                    if let Some(backup_file) = backed_up.iter().find(|x| x.file == delete.id) {
                        let deleted_count = mc.remove_backup_files_for_media(&backup.id, &backup_file.file, None, None, &ConnectedUser::ServerAdmin).await?;
                        log_info(crate::tools::log::LogServiceType::Scheduler, format!("Deleted {} files from backup: {} ({})", deleted_count, backup_file.file, human_bytes(backup_file.size as f64)));
                    }
                }
                let mut done_size = 0u64;
                let mut current = 0u64;
                for backup_media in backup_medias {
                    current += 1;
                    if backed_up.iter().any(|b| b.file == backup_media.id && &b.backup == &backup.id) { // should also check sourcehash in the future for modifications
                        log_info(crate::tools::log::LogServiceType::Scheduler, format!("Duplicate backup file found for library {} file: {} ({})", library_id, backup_media.id, human_bytes(backup_media.size.unwrap_or(0) as f64)));
                    } else {
                        let message = BackupProcessStatus::new_from_backup(&backup, total, current, total_size, done_size) ;
                        mc.set_backup_status(message).await;
                        log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backing up library {} file: {} ({})", library_id, backup_media.id, human_bytes(backup_media.size.unwrap_or(0) as f64)));

                        let backedup = backup_file(&backup_media, &backup, library_id, &mc).await;

                        if let Err(e) = backedup {
                            log_error(crate::tools::log::LogServiceType::Scheduler, format!("Backing up library {} file {} failed with error: {}", library_id, backup_media.id, e.to_string()));
                            let error = BackupError::new(backup.id.clone(), library_id.to_string(), backup_media.id.clone(), e);
                            mc.add_backup_error(error, &ConnectedUser::ServerAdmin).await?;
                        }
                    }
                    done_size += backup_media.size.unwrap_or(0);
                    log_info(crate::tools::log::LogServiceType::Scheduler, format!("Remaining backup size: {}", total_size - done_size));
                }
                    
                let backup_files_infos = mc.get_backup_files_infos(&backup.id, &ConnectedUser::ServerAdmin).await?;

            
            
                let server_db_path = get_server_file_path_array(vec![&"dbs", &format!("db-{}.db", library_id)]).await.map_err(|_| RsError::Error("Unable to get database path for backup".to_string()))?;
                let db_backup = mc.upload_backup_path(backup.clone(), "db", server_db_path, format!("db-{}", now().format("%Y%m%d%H%M"))).await?;

                
                let delete_dbs_before = now().add(Duration::days(-7))?.timestamp_millis();
                let removed = mc.remove_backup_files_for_media(&backup.id, "db", Some(delete_dbs_before), Some(db_backup.id.clone()), &ConnectedUser::ServerAdmin).await?;
                log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backup removed {} dbs backup", removed));
                //println!("db backup: {:?}", db_backup);
                let bakcup_update = BackupForUpdate {
                    size: backup_files_infos.size,
                    last: Some(now().timestamp_millis()),
                    ..Default::default()
                };
                mc.update_backup(&backup.id, bakcup_update, &ConnectedUser::ServerAdmin).await?;
                let message = BackupProcessStatus::new_from_backup_done(&backup) ;
                mc.set_backup_status(message).await;
            } else {
                let message = BackupProcessStatus::new_from_backup(&backup, 2, 0, 0, 0) ;
                mc.set_backup_status(message).await;

                let server_db_path = get_server_file_path_array(vec![&"dbs", "database.db"]).await.map_err(|_| RsError::Error("Unable to get database path for backup".to_string()))?;
                let db_backup = mc.upload_backup_path(backup.clone(), "db", server_db_path, format!("db-{}", now().format("%Y%m%d%H%M"))).await?;
                let delete_dbs_before = now().add(Duration::days(-7))?.timestamp_millis();
                let removed = mc.remove_backup_files_for_media(&backup.id, "db", Some(delete_dbs_before), Some(db_backup.id.clone()), &ConnectedUser::ServerAdmin).await?;
                log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backup removed {} dbs backup", removed));
                
                let message = BackupProcessStatus::new_from_backup(&backup, 2, 1, 0, 0) ;
                mc.set_backup_status(message).await;

                let server_db_path = get_server_file_path_array(vec!["config.json"]).await.map_err(|_| RsError::Error("Unable to get config.json path for backup".to_string()))?;
                let db_backup = mc.upload_backup_path(backup.clone(), "config", server_db_path, format!("config-{}", now().format("%Y%m%d%H%M"))).await?;
                let delete_dbs_before = now().add(Duration::days(-7))?.timestamp_millis();
                let removed = mc.remove_backup_files_for_media(&backup.id, "config", Some(delete_dbs_before), Some(db_backup.id.clone()), &ConnectedUser::ServerAdmin).await?;
                log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backup removed {} config backup", removed));

                let message = BackupProcessStatus::new_from_backup_done(&backup) ;
                mc.set_backup_status(message).await;
            }


        }
        
        log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backed up all"));
        Ok(())
    }
}



async fn backup_file(backup_media: &MediaBackup, backup: &Backup, library_id: &str, mc: &ModelController) -> RsResult<BackupFile> {
    let id = nanoid!();
    let backedup = mc.upload_backup_media(&backup.id, library_id, &backup_media.id, Some(id), &ConnectedUser::ServerAdmin).await?;
    mc.add_backup_file(backedup.clone(), &ConnectedUser::ServerAdmin).await?;

    Ok(backedup)

}