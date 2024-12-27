use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;

use crate::{error::RsError, model::medias::MediaQuery, tools::{clock::now, scheduler::backup}};

use super::{library, media::Media, ElementAction};


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
	pub name: String,
	pub source: String,
    pub plugin: Option<String>,
    pub credentials: Option<String>,
    pub library: Option<String>,
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
	pub library: Option<String>,
    pub file: String,
    pub id: String,
    pub path: String,
    pub hash: String,
    pub sourcehash: String,
    pub size: u64,
    pub modified: i64,
    pub added: i64,
    pub iv: Option<String>,
    pub thumb_size: Option<u64>,
    pub info_size: Option<u64>,
    pub error: Option<String>,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackupError {
    pub id: String,
    pub backup: String,
	pub library: String,
    pub file: String,
    pub date: i64,
    pub error: String,
}

impl BackupError {
    pub fn new(backup: String, library: String, file: String, error: RsError) -> Self {
        BackupError { id: nanoid!(), backup, library, file, date: now().timestamp_millis(), error: error.to_string() }
    }
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
pub struct BackupProcessStatus {
    pub library: Option<String>,
    pub backup: String,
    pub status: BackupStatus,
    pub time: i64,
    pub total: u64,
    pub current: u64,
    pub total_size: u64,
    pub current_size: u64,

    pub estimated_remaining_seconds: Option<u64>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct BackupWithStatus {
    pub backup: Backup,
    pub status: Option<BackupProcessStatus>,
}

impl BackupProcessStatus {
    pub fn new_from_backup(backup: &Backup, total: u64, current: u64, total_size: u64, current_size: u64) -> Self {
        let backup = backup.clone();
        BackupProcessStatus {
            library: backup.library,
            backup: backup.id,
            status: BackupStatus::InProgress,
            time: now().timestamp_millis(),
            total,
            current,
            total_size,
            current_size,
            
            estimated_remaining_seconds: None
        }
    }
    pub fn new_from_backup_idle(backup: &Backup) -> Self {
        let backup = backup.clone();
        BackupProcessStatus {
            library: backup.library,
            backup: backup.id,
            status: BackupStatus::Idle,
            time: 0,
            total: 0,
            current: 0,
            total_size: 0,
            current_size: 0,

            estimated_remaining_seconds: None
        }
    }

    pub fn new_from_backup_done(backup: &Backup) -> Self {
        let backup = backup.clone();
        BackupProcessStatus {
            library: backup.library,
            backup: backup.id,
            status: BackupStatus::Done,
            time: 0,
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
    pub name: Option<String>,
    pub backup: String,
	pub library: Option<String>,
    pub file: String,
    pub id: String,
    pub size: Option<u64>,
    pub status: BackupStatus,
    pub progress: u64,
    pub error: Option<String>,

    pub estimated_remaining_seconds: Option<u64>
}

impl BackupFileProgress {
    pub fn new_from(backup: &Backup, media: &Media, id: String, status: BackupStatus, progress: u64, error: Option<String>) -> Self {
        BackupFileProgress {
            name: Some(media.name.clone()),
            backup: backup.id.clone(),
            library: backup.library.clone(),
            file: media.id.clone(),
            status,
            id,
            size: media.size.clone(),
            progress,
            error,
            estimated_remaining_seconds: None,
        }
    }

    pub fn new(file: String, backup: String, library: Option<String>, name: Option<String>, size: Option<u64>, id: String, status: BackupStatus, progress: u64, error: Option<String>) -> Self {
        BackupFileProgress {
            name,
            backup,
            library,
            file,
            status,
            id,
            size,
            progress,
            error,
            estimated_remaining_seconds: None,
        }
    }

    pub fn new_light(backup_id: String, file_id: String, id: String, size: Option<u64>, status: BackupStatus, progress: u64, error: Option<String>) -> Self {
        BackupFileProgress {
            name: None,
            backup: backup_id,
            library: None,
            file: file_id,
            status,
            id,
            size,
            progress,
            error,
            estimated_remaining_seconds: None,
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct BackupMessage {
    pub action: ElementAction,
    pub backup: BackupWithStatus,
}
