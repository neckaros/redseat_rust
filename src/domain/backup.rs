use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;

use crate::{model::medias::MediaQuery, tools::clock::now};


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
	pub source: String,
    pub plugin: Option<String>,
    pub credentials: Option<String>,
    pub library: String,
    pub path: String,
    pub schedule: Option<String>,
    pub filter: Option<MediaQuery>,
    pub last: Option<i64>,
    pub password: Option<String>,
    pub size: u64,
}




#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackupFile {
    pub backup: String,
	pub library: String,
    pub file: String,
    pub id: String,
    pub path: String,
    pub hash: String,
    pub sourcehash: String,
    pub size: u64,
    pub date: i64,
    pub iv: Option<String>,
    pub thumb_size: Option<u64>,
    pub info_size: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display,EnumString, Default)]
#[strum(serialize_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub enum BackupStatus {
    InProgress,
    Done,
    Error,
    #[default]
    Idle
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct BackupStatusMessage {
    pub library: String,
    pub backup: String,
    pub status: BackupStatus,
    pub time: i64,
    pub files: Vec<BackupFileProgress>,
    pub total: u64,
    pub current: u64,
    pub total_size: u64,
    pub current_size: u64,

    pub estimated_remaining_seconds: Option<u64>
}

impl BackupStatusMessage {
    pub fn new_from_backup(backup: &Backup, files: Vec<BackupFileProgress>, total: u64, current: u64, total_size: u64, current_size: u64) -> Self {
        let backup = backup.clone();
        BackupStatusMessage {
            library: backup.library,
            backup: backup.id,
            status: BackupStatus::InProgress,
            time: now().timestamp_millis(),
            files,
            total,
            current,
            total_size,
            current_size,
            
            estimated_remaining_seconds: None
        }
    }
    pub fn new_from_backup_idle(backup: &Backup) -> Self {
        let backup = backup.clone();
        BackupStatusMessage {
            library: backup.library,
            backup: backup.id,
            status: BackupStatus::Idle,
            time: 0,
            files: vec![],
            total: 0,
            current: 0,
            total_size: 0,
            current_size: 0,

            estimated_remaining_seconds: None
        }
    }

    pub fn new_from_backup_done(backup: &Backup) -> Self {
        let backup = backup.clone();
        BackupStatusMessage {
            library: backup.library,
            backup: backup.id,
            status: BackupStatus::Done,
            time: 0,
            files: vec![],
            total: 0,
            current: 0,
            total_size: 0,
            current_size: 0,

            estimated_remaining_seconds: None
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackupFileProgress {
    pub name: String,
    pub backup: String,
	pub library: String,
    pub file: String,
    pub id: String,
    pub size: u64,
    pub progress: u64,
    pub error: Option<String>,

    pub estimated_remaining_seconds: Option<u64>
}