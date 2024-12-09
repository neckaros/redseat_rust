use axum::{async_trait, Error};
use chrono::DateTime;
use human_bytes::human_bytes;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::domain::movie::Movie;
use crate::domain::serie::{Serie, SerieStatus};
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
            let media_query = backup.filter.unwrap_or_default();
            let backup_medias = mc.get_medias_to_backup(&backup.library, backup.last.unwrap_or(i64::min_value()), media_query, &ConnectedUser::ServerAdmin).await?;
            let total_size: u64 = backup_medias.iter().filter_map(|backup| backup.size).sum();
            log_info(crate::tools::log::LogServiceType::Scheduler, format!("Backing up {} medias for size of {} from library {}", backup_medias.len(), human_bytes(total_size as f64), backup.library));
            //println!("medias backups: {:?}", backup_medias);
                    
                
                
            
        }
                
        log_info(crate::tools::log::LogServiceType::Scheduler, format!("Refreshed all"));
        Ok(())
    }
}