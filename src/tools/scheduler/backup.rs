use crate::domain::backup::{
    Backup, BackupError, BackupFile, BackupFileProgress, BackupProcessStatus,
};
use crate::domain::deleted;
use crate::domain::movie::Movie;
use crate::domain::serie::{Serie, SerieStatus};
use crate::error::RsError;
use crate::model::backups::BackupForUpdate;
use crate::model::deleted::DeletedQuery;
use crate::model::episodes::{EpisodeForUpdate, EpisodeQuery};
use crate::model::movies::MovieQuery;
use crate::model::series::SerieForUpdate;
use crate::model::store::sql::backups::BackupMediaState;
use crate::model::store::sql::library::medias::MediaBackup;
use crate::plugins::sources::path_provider::PathProvider;
use crate::server::get_server_file_path_array;
use crate::tools::clock::{now, Clock};
use crate::{
    domain::library,
    error::RsResult,
    model::{series::SerieQuery, users::ConnectedUser, ModelController},
    plugins::sources::Source,
    tools::{
        clock::UtcDate,
        log::{log_error, log_info},
    },
};
use axum::{async_trait, Error};
use chrono::{DateTime, Duration};
use human_bytes::human_bytes;
use nanoid::nanoid;
use rs_plugin_common_interfaces::ElementType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::RsSchedulerTask;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupTask {
    pub specific_backup: Option<String>,
}

#[async_trait]
impl RsSchedulerTask for BackupTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        let connected_user = &ConnectedUser::ServerAdmin;
        let backups = mc.get_backups(&connected_user).await?;
        let backups = if let Some(specific_backup) = &self.specific_backup {
            backups
                .into_iter()
                .filter(|l| &l.id == specific_backup)
                .collect()
        } else {
            backups
        };

        if let Some(specific_backup) = &self.specific_backup {
            if backups.is_empty() {
                return Err(RsError::BackupProcessNotFound(specific_backup.clone()));
            }
        }

        for backup in backups {
            if !mc.try_start_backup(&backup).await {
                log_info(
                    crate::tools::log::LogServiceType::Scheduler,
                    format!("Backup {} is already in progress", backup.id),
                );
                continue;
            }

            let result: RsResult<()> = async {
                if let Some(library_id) = &backup.library {
                    let mut backup_files_infos = mc
                        .get_backup_files_infos(&backup.id, &ConnectedUser::ServerAdmin)
                        .await?;
                    let library =
                        mc.get_internal_library(library_id)
                            .await?
                            .ok_or(RsError::Error(format!(
                                "Unable to find library {} for backup {}",
                                library_id, backup.id
                            )))?;

                    if library.source != "virtual" {
                        let backed_up = mc.get_backup_media_states(&backup.id, library_id).await?;
                        let media_query = backup.filter.clone().unwrap_or_default();
                        let backup_medias = pending_backup_medias(
                            mc.get_medias_to_backup(
                                library_id,
                                media_query,
                                &ConnectedUser::ServerAdmin,
                            )
                            .await?,
                            &backed_up,
                        );
                        let total = backup_medias.len() as u64;
                        let total_size: u64 =
                            backup_medias.iter().filter_map(|backup| backup.size).sum();
                        log_info(
                            crate::tools::log::LogServiceType::Scheduler,
                            format!(
                                "Backing up {} medias for size of {} from library {}",
                                backup_medias.len(),
                                human_bytes(total_size as f64),
                                library_id
                            ),
                        );
                        //println!("medias backups: {:?}", backup_medias);

                        let deleted = mc
                            .get_deleted(
                                library_id,
                                DeletedQuery {
                                    after: backup.last,
                                    kind: Some(ElementType::Media),
                                    ..Default::default()
                                },
                                &ConnectedUser::ServerAdmin,
                            )
                            .await?;
                        for delete in deleted {
                            if let Some(backup_file) =
                                backed_up.iter().find(|x| x.file == delete.id)
                            {
                                let deleted_count = mc
                                    .remove_backup_files_for_media(
                                        &backup.id,
                                        &backup_file.file,
                                        None,
                                        None,
                                        &ConnectedUser::ServerAdmin,
                                    )
                                    .await?;
                                log_info(
                                    crate::tools::log::LogServiceType::Scheduler,
                                    format!(
                                        "Deleted {} files from backup: {}",
                                        deleted_count, backup_file.file
                                    ),
                                );
                            }
                        }
                        let mut done_size = 0u64;
                        let mut current = 0u64;
                        for backup_media in backup_medias {
                            current += 1;
                            let message = BackupProcessStatus::new_from_backup(
                                &backup, total, current, total_size, done_size,
                            );
                            mc.set_backup_status(message).await?;
                            log_info(
                                crate::tools::log::LogServiceType::Scheduler,
                                format!(
                                    "Backing up library {} file: {} ({})",
                                    library_id,
                                    backup_media.id,
                                    human_bytes(backup_media.size.unwrap_or(0) as f64)
                                ),
                            );

                            let backedup =
                                backup_file(&backup_media, &backup, library_id, &mc).await;

                            if let Err(e) = backedup {
                                log_error(
                                    crate::tools::log::LogServiceType::Scheduler,
                                    format!(
                                        "Backing up library {} file {} failed with error: {}",
                                        library_id,
                                        backup_media.id,
                                        e.to_string()
                                    ),
                                );
                                let error = BackupError::new(
                                    backup.id.clone(),
                                    library_id.to_string(),
                                    backup_media.id.clone(),
                                    e,
                                );
                                mc.add_backup_error(error, &ConnectedUser::ServerAdmin)
                                    .await?;
                            }
                            done_size += backup_media.size.unwrap_or(0);
                            log_info(
                                crate::tools::log::LogServiceType::Scheduler,
                                format!("Remaining backup size: {}", total_size - done_size),
                            );
                        }

                        backup_files_infos = mc
                            .get_backup_files_infos(&backup.id, &ConnectedUser::ServerAdmin)
                            .await?;
                    }

                    let database_snapshot = mc.create_database_snapshot(Some(library_id)).await?;
                    let db_backup = mc
                        .upload_backup_path(
                            backup.clone(),
                            "db",
                            database_snapshot.path().to_path_buf(),
                            format!("db-{}", now().format("%Y%m%d%H%M")),
                            Some(library),
                        )
                        .await?;

                    let delete_dbs_before = now().add(Duration::days(-7))?.timestamp_millis();
                    let removed = mc
                        .remove_backup_files_for_media(
                            &backup.id,
                            "db",
                            Some(delete_dbs_before),
                            Some(db_backup.id.clone()),
                            &ConnectedUser::ServerAdmin,
                        )
                        .await?;
                    log_info(
                        crate::tools::log::LogServiceType::Scheduler,
                        format!("Backup removed {} dbs backup", removed),
                    );
                    //println!("db backup: {:?}", db_backup);
                    let bakcup_update = BackupForUpdate {
                        size: backup_files_infos.size,
                        last: Some(now().timestamp_millis()),
                        ..Default::default()
                    };
                    mc.update_backup(&backup.id, bakcup_update, &ConnectedUser::ServerAdmin)
                        .await?;
                } else {
                    let message = BackupProcessStatus::new_from_backup(&backup, 2, 0, 0, 0);
                    mc.set_backup_status(message).await?;

                    let database_snapshot = mc.create_database_snapshot(None).await?;
                    let db_backup = mc
                        .upload_backup_path(
                            backup.clone(),
                            "db",
                            database_snapshot.path().to_path_buf(),
                            format!("db-{}", now().format("%Y%m%d%H%M")),
                            None,
                        )
                        .await?;
                    let delete_dbs_before = now().add(Duration::days(-7))?.timestamp_millis();
                    let removed = mc
                        .remove_backup_files_for_media(
                            &backup.id,
                            "db",
                            Some(delete_dbs_before),
                            Some(db_backup.id.clone()),
                            &ConnectedUser::ServerAdmin,
                        )
                        .await?;
                    log_info(
                        crate::tools::log::LogServiceType::Scheduler,
                        format!("Backup removed {} dbs backup", removed),
                    );

                    let message = BackupProcessStatus::new_from_backup(&backup, 2, 1, 0, 0);
                    mc.set_backup_status(message).await?;

                    let server_db_path = get_server_file_path_array(vec!["config.json"])
                        .await
                        .map_err(|_| {
                            RsError::Error("Unable to get config.json path for backup".to_string())
                        })?;
                    let db_backup = mc
                        .upload_backup_path(
                            backup.clone(),
                            "config",
                            server_db_path,
                            format!("config-{}", now().format("%Y%m%d%H%M")),
                            None,
                        )
                        .await?;
                    let delete_dbs_before = now().add(Duration::days(-7))?.timestamp_millis();
                    let removed = mc
                        .remove_backup_files_for_media(
                            &backup.id,
                            "config",
                            Some(delete_dbs_before),
                            Some(db_backup.id.clone()),
                            &ConnectedUser::ServerAdmin,
                        )
                        .await?;
                    log_info(
                        crate::tools::log::LogServiceType::Scheduler,
                        format!("Backup removed {} config backup", removed),
                    );

                    let backup_files_infos = mc
                        .get_backup_files_infos(&backup.id, &ConnectedUser::ServerAdmin)
                        .await?;
                    mc.update_backup(
                        &backup.id,
                        BackupForUpdate {
                            size: backup_files_infos.size,
                            last: Some(now().timestamp_millis()),
                            ..Default::default()
                        },
                        &ConnectedUser::ServerAdmin,
                    )
                    .await?;
                }

                Ok(())
            }
            .await;

            match result {
                Ok(()) => {
                    mc.set_backup_status(BackupProcessStatus::new_from_backup_done(&backup))
                        .await?;
                }
                Err(error) => {
                    mc.set_backup_status(BackupProcessStatus::new_from_backup_error(&backup))
                        .await?;
                    return Err(error);
                }
            }
        }

        log_info(
            crate::tools::log::LogServiceType::Scheduler,
            "Backed up all configured targets".to_string(),
        );
        Ok(())
    }
}

fn pending_backup_medias(
    medias: Vec<MediaBackup>,
    backed_up: &[BackupMediaState],
) -> Vec<MediaBackup> {
    let latest_backups = backed_up
        .iter()
        .fold(HashMap::<&str, i64>::new(), |mut latest, file| {
            latest
                .entry(file.file.as_str())
                .and_modify(|modified| *modified = (*modified).max(file.modified))
                .or_insert(file.modified);
            latest
        });

    medias
        .into_iter()
        .filter(|media| {
            latest_backups
                .get(media.id.as_str())
                .map(|modified| *modified < media.modified)
                .unwrap_or(true)
        })
        .collect()
}

async fn backup_file(
    backup_media: &MediaBackup,
    backup: &Backup,
    library_id: &str,
    mc: &ModelController,
) -> RsResult<BackupFile> {
    let id = nanoid!();
    let backedup = mc
        .upload_backup_media(
            &backup.id,
            library_id,
            &backup_media.id,
            Some(id),
            &ConnectedUser::ServerAdmin,
        )
        .await?;
    mc.add_backup_file(backedup.clone(), &ConnectedUser::ServerAdmin)
        .await?;

    Ok(backedup)
}

#[cfg(test)]
mod tests {
    use super::pending_backup_medias;
    use crate::model::store::sql::backups::BackupMediaState;
    use crate::model::store::sql::library::medias::MediaBackup;

    #[test]
    fn selects_missing_and_stale_media_for_backup() {
        let medias = vec![
            MediaBackup {
                id: "current".to_string(),
                name: "current".to_string(),
                size: None,
                modified: 10,
            },
            MediaBackup {
                id: "stale".to_string(),
                name: "stale".to_string(),
                size: None,
                modified: 20,
            },
            MediaBackup {
                id: "missing".to_string(),
                name: "missing".to_string(),
                size: None,
                modified: 30,
            },
        ];
        let backups = vec![
            BackupMediaState {
                file: "current".to_string(),
                modified: 10,
            },
            BackupMediaState {
                file: "stale".to_string(),
                modified: 15,
            },
        ];

        let pending = pending_backup_medias(medias, &backups);
        assert_eq!(
            pending
                .into_iter()
                .map(|media| media.id)
                .collect::<Vec<_>>(),
            vec!["stale", "missing"]
        );
    }
}
