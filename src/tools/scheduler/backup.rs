use axum::{async_trait, Error};
use chrono::DateTime;
use human_bytes::human_bytes;
use nanoid::nanoid;
use rs_plugin_common_interfaces::ElementType;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::domain::backup::{BackupFileProgress, BackupStatusMessage};
use crate::domain::deleted;
use crate::domain::movie::Movie;
use crate::domain::serie::{Serie, SerieStatus};
use crate::model::backups::BackupForUpdate;
use crate::model::deleted::DeletedQuery;
use crate::model::episodes::{EpisodeForUpdate, EpisodeQuery};
use crate::model::movies::MovieQuery;
use crate::model::series::SerieForUpdate;
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
            let backup_files_infos = mc.get_backup_files_infos(&backup.id, &ConnectedUser::ServerAdmin).await?;
          

            let media_query = backup.filter.clone().unwrap_or_default();
            let backup_medias = mc.get_medias_to_backup(&backup.library, backup_files_infos.max_date.unwrap_or(i64::min_value()), media_query, &ConnectedUser::ServerAdmin).await?;
            let total = backup_medias.len() as u64;
            let total_size: u64 = backup_medias.iter().filter_map(|backup| backup.size).sum();
            log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backing up {} medias for size of {} from library {}", backup_medias.len(), human_bytes(total_size as f64), backup.library));
            //println!("medias backups: {:?}", backup_medias);
                    
            let deleted = mc.get_deleted(&backup.library, DeletedQuery { after: backup.last, kind: Some(ElementType::Media), ..Default::default() }, &ConnectedUser::ServerAdmin).await?; 
            let backed_up = mc.get_library_backup_files(&backup.library, &ConnectedUser::ServerAdmin).await?;
            


            for delete in deleted {
                if let Some(backup_file) = backed_up.iter().find(|x| x.file == delete.id) {
                    let deleted_count = mc.remove_backup_files_for_media(&backup.id ,&backup.library, &backup_file.file, &ConnectedUser::ServerAdmin).await?;
                    log_info(crate::tools::log::LogServiceType::Scheduler, format!("Deleted {} files from backup: {} ({})", deleted_count, backup_file.file, human_bytes(backup_file.size as f64)));
                }
            }
            let mut done_size = 0u64;
            let mut current = 0u64;
            for backup_media in backup_medias {
                current += 1;
                if backed_up.iter().any(|b| b.file == backup_media.id && b.sourcehash == backup_media.hash) {
                    log_info(crate::tools::log::LogServiceType::Scheduler, format!("Duplicate backup file found for library {} file: {} ({})", backup.library, backup_media.id, human_bytes(backup_media.size.unwrap_or(0) as f64)));
                } else {
                    let id = nanoid!();
                    let progress_file = BackupFileProgress { name: backup_media.name.to_owned(), backup: backup.id.to_owned(), library: backup.library.to_owned(), file: backup_media.id.clone(), id: id.clone(), size: backup_media.size.unwrap_or_default(), progress: 0, error: None, estimated_remaining_seconds: None };
                    let message = BackupStatusMessage::new_from_backup(&backup, vec![progress_file], total, current, total_size, done_size) ;
                    mc.send_backup_status(message);
                    log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backing up library {} file: {} ({})", backup.library, backup_media.id, human_bytes(backup_media.size.unwrap_or(0) as f64)));
                    let backedup = mc.upload_backup_media(&backup.id, &backup_media.id, Some(id), &ConnectedUser::ServerAdmin).await?;
                    mc.add_backup_file(backedup, &ConnectedUser::ServerAdmin).await?;
                }
                done_size += backup_media.size.unwrap_or(0);
                log_info(crate::tools::log::LogServiceType::Scheduler, format!("Remaining backup size: {}", total_size - done_size));
            }
                
            let backup_files_infos = mc.get_backup_files_infos(&backup.id, &ConnectedUser::ServerAdmin).await?;

            let bakcup_update = BackupForUpdate {
                size: backup_files_infos.size,
                last: Some(now().timestamp_millis()),
                ..Default::default()
            };
            mc.update_backup(&backup.id, bakcup_update, &ConnectedUser::ServerAdmin).await?;
            let message = BackupStatusMessage::new_from_backup_done(&backup) ;
            mc.send_backup_status(message);

        }
        
        log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backed up all"));
        Ok(())
    }
}