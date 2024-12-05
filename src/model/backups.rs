


use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;


use crate::domain::{backup::{Backup, BackupFile}, library::LibraryRole};

use super::{error::{Error, Result}, users::{ConnectedUser, UserRole}, ModelController};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupForAdd {
	pub source: String,
    pub credentials: Option<String>,
    pub plugin: Option<String>,
    pub library: String,
    pub path: String,
    pub schedule: Option<String>,
    pub filter: Option<Value>,
    pub last: Option<u64>,
    pub password: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupForUpdate {
	pub source: Option<String>,
	pub credentials: Option<String>,
    pub plugin: Option<String>,
	pub library: Option<String>,
    pub path: Option<String>,
    pub schedule: Option<String>,
    pub filter: Option<Value>,
    pub last: Option<u64>,
    pub password: Option<String>,
    pub size: Option<u64>,
}



impl ModelController {

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

    pub async fn get_backup_file(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> Result<Vec<BackupFile>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
		let credential = self.store.get_backup_file(&library_id, &media_id).await?;
		Ok(credential)
	}
}
