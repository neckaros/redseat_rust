use nanoid::nanoid;
use rusqlite::{params, params_from_iter, OptionalExtension, Row, ToSql};

use crate::{domain::backup::{Backup, BackupError, BackupFile}, error::RsError, model::{backups::BackupForUpdate, store::SqliteStore}};
use super::Result;

pub struct BackupInfos {
    pub max_date: Option<i64>,
    pub size: Option<u64>,
}

const BACKUP_FILE_QUERY_ELEMENTS: &str = "backup, library, file, id, path, hash, sourcehash, size, modified, added, iv, infoSize, thumbsize, error";

impl SqliteStore {
  
    fn backup_file_from_row(row: &Row) -> rusqlite::Result<BackupFile> {
        Ok(BackupFile {
            backup: row.get(0)?,
            library:row.get(1)?,
            file:row.get(2)?,
            id:row.get(3)?,
            path:row.get(4)?,
            hash:row.get(5)?,
            sourcehash:row.get(6)?,
            size:row.get(7)?,
            modified:row.get(8)?,
            added:row.get(9)?,
            iv:row.get(10)?,
            info_size:row.get(11)?,
            thumb_size: row.get(12)?,
            error:row.get(13)?,
        })
    }

    fn backup_error_from_row(row: &Row) -> rusqlite::Result<BackupError> {
        Ok(BackupError {
            id: row.get(0)?,
            backup: row.get(1)?,
            library:row.get(2)?,
            file:row.get(3)?,
            date:row.get(4)?,
            error:row.get(5)?,
        })
    }
    
    
    pub async fn get_backups(&self) -> Result<Vec<Backup>> {
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, source, credentials, library, path, schedule, filter, last, password, size, plugin, name  FROM Backups")?;
            let rows = query.query_map(
            [],
            |row| {
                Ok(Backup {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    credentials:  row.get(2)?,
                    library:  row.get(3)?,
                    path:  row.get(4)?,
                    schedule:  row.get(5)?,
                    filter:  row.get(6)?,
                    last:  row.get(7)?,
                    password:  row.get(8)?,
                    size:  row.get(9)?,
                    plugin:  row.get(10)?,
                    name:  row.get(11)?,
                    
                })
            },
            )?;
            let backups:Vec<Backup> = rows.collect::<std::result::Result<Vec<Backup>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }

    pub async fn get_backup(&self, credential_id: &str) -> Result<Option<Backup>> {
        let credential_id = credential_id.to_string();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, source, credentials, library, path, schedule, filter, last, password, size, plugin, name  FROM Backups WHERE id = ?")?;
            let row = query.query_row(
            [credential_id],
            |row| {
                Ok(Backup {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    credentials:  row.get(2)?,
                    library:  row.get(3)?,
                    path:  row.get(4)?,
                    schedule:  row.get(5)?,
                    filter:  row.get(6)?,
                    last:  row.get(7)?,
                    password:  row.get(8)?,
                    size:  row.get(9)?,
                    plugin:  row.get(10)?,
                    name:  row.get(11)?,
                })
            },
            ).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }



    pub async fn update_backup(&self, credential_id: &str, update: BackupForUpdate) -> Result<()> {
        let credential_id = credential_id.to_string();
        self.server_store.call( move |conn| { 
            let mut columns: Vec<String> = Vec::new();
            let mut values: Vec<Box<dyn ToSql>> = Vec::new();
            super::add_for_sql_update(update.source, "source", &mut columns, &mut values);
            super::add_for_sql_update(update.plugin, "plugin", &mut columns, &mut values);
            super::add_for_sql_update(update.credentials, "credentials", &mut columns, &mut values);
            super::add_for_sql_update(update.library, "library", &mut columns, &mut values);
            super::add_for_sql_update(update.path, "path", &mut columns, &mut values);
            super::add_for_sql_update(update.schedule, "schedule", &mut columns, &mut values);
            super::add_for_sql_update(update.filter, "filter", &mut columns, &mut values);
            super::add_for_sql_update(update.last, "last", &mut columns, &mut values);
            super::add_for_sql_update(update.password, "password", &mut columns, &mut values);
            super::add_for_sql_update(update.size, "size", &mut columns, &mut values);
            super::add_for_sql_update(update.name, "name", &mut columns, &mut values);


            if columns.len() > 0 {
                values.push(Box::new(credential_id));
                let update_sql = format!("UPDATE Backups SET {} WHERE id = ?", columns.join(", "));
                conn.execute(&update_sql, params_from_iter(values))?;
            }
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn add_backup(&self, backup: Backup) -> Result<()> {
        self.server_store.call( move |conn| { 

            conn.execute("INSERT INTO Backups (id, source, credentials, library, path, schedule, filter, last, password, size, plugin, name)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
                backup.id,
                backup.source,
                backup.credentials,
                backup.library,
                backup.path,
                backup.schedule,
                backup.filter,
                backup.last,
                backup.password,
                backup.size,
                backup.plugin,
                backup.name,
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_backup(&self, credential_id: String) -> Result<()> {
        self.server_store.call( move |conn| { 
            conn.execute("DELETE FROM Backups WHERE id = ?", &[&credential_id])?;
            Ok(())
        }).await?;
        Ok(())
    }



    // BACKUP FILE ============================
    pub async fn get_backup_file(&self, backup_file_id: &str) -> Result<Option<BackupFile>> {
        let backup_file_id = backup_file_id.to_owned();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare(&format!("SELECT {BACKUP_FILE_QUERY_ELEMENTS} FROM Backups_Files WHERE id = ?"))?;
            let row = query.query_row(
            [backup_file_id],Self::backup_file_from_row,
            ).optional()?;
            Ok(row)
           
        }).await?;
        Ok(row)
    }

    /// Recover all the existing files for a backup
    pub async fn get_backup_backup_files(&self, backup_id: &str) -> Result<Vec<BackupFile>> {
        let backup_id = backup_id.to_owned();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare(&format!("SELECT {BACKUP_FILE_QUERY_ELEMENTS} FROM Backups_Files WHERE backup = ? ORDER BY added DESC"))?;
            let rows = query.query_map(
            [backup_id],Self::backup_file_from_row,
            )?;
            let files:Vec<BackupFile> = rows.collect::<std::result::Result<Vec<BackupFile>, rusqlite::Error>>()?; 
            Ok(files)
        }).await?;
        Ok(row)
    }

    /// For a specific media id get all the files for a specific backup
    pub async fn get_backup_media_backup_files(&self, backup_id: &str, media_id: &str) -> Result<Vec<BackupFile>> {
        let media_id = media_id.to_owned();
        let backup_id = backup_id.to_owned();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare(&format!("SELECT {BACKUP_FILE_QUERY_ELEMENTS} FROM Backups_Files WHERE file = ? and backup = ? ORDER BY added DESC"))?;
            let rows = query.query_map(
            [media_id, backup_id],Self::backup_file_from_row,
            )?;
            let files:Vec<BackupFile> = rows.collect::<std::result::Result<Vec<BackupFile>, rusqlite::Error>>()?; 
            Ok(files)
        }).await?;
        Ok(row)
    }

    /// For a specific media id get all the files whatever the backup
    pub async fn get_library_media_backup_files(&self, library_id: &str, media_id: &str) -> Result<Vec<BackupFile>> {
        let media_id = media_id.to_owned();
        let library_id = library_id.to_owned();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare(&format!("SELECT {BACKUP_FILE_QUERY_ELEMENTS} FROM Backups_Files WHERE file = ? and library = ? ORDER BY added DESC"))?;
            let rows = query.query_map(
            [media_id, library_id],Self::backup_file_from_row,
            )?;
            let files:Vec<BackupFile> = rows.collect::<std::result::Result<Vec<BackupFile>, rusqlite::Error>>()?; 
            Ok(files)
        }).await?;
        Ok(row)
    }

    /// Get all the backup files for a library, whatever the backup
    pub async fn get_library_backup_files(&self, library_id: &str) -> Result<Vec<BackupFile>> {
        let library_id = library_id.to_owned();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare(&format!("SELECT {BACKUP_FILE_QUERY_ELEMENTS} FROM Backups_Files WHERE library = ? and file <> 'db' ORDER BY added DESC"))?;
            let rows = query.query_map(
            [library_id],
            Self::backup_file_from_row,
            )?;
            let files:Vec<BackupFile> = rows.collect::<std::result::Result<Vec<BackupFile>, rusqlite::Error>>()?; 
            Ok(files)
        }).await?;
        Ok(row)
    }


    pub async fn get_backup_files_infos(&self, backup_id: &str) -> Result<BackupInfos> {
        let backup_id = backup_id.to_owned();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT MAX(modified), SUM(size) FROM Backups_Files WHERE backup = ?  and file <> 'db'")?;

            let row: BackupInfos = query.query_row(
                params![backup_id], |row| Ok(BackupInfos {
                    max_date: row.get(0)?,
                    size: row.get(1)?
                }),
            )?;
            
            Ok(row)
        }).await?;
        Ok(row)
    }


    
    
    pub async fn add_backup_file(&self, backup: BackupFile) -> Result<()> {
        self.server_store.call( move |conn| { 
            conn.execute("INSERT INTO Backups_Files (backup, library, file, id, path, hash, sourcehash, size, modified, added, iv, thumbsize, infoSize, error)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![
                backup.backup,
                backup.library,
                backup.file,
                backup.id,
                backup.path,
                backup.hash,
                backup.sourcehash,
                backup.size,
                backup.modified,
                backup.added,
                backup.iv,
                backup.thumb_size,
                backup.info_size,
                backup.error
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_backup_file(&self, id: String) -> Result<()> {
        self.server_store.call( move |conn| { 
            conn.execute("DELETE FROM Backups_Files WHERE id = ?", &[&id])?;
            Ok(())
        }).await?;
        Ok(())
    }


    //ERRORS
     pub async fn get_backup_error(&self, backup_error_id: &str) -> Result<Option<BackupError>> {
        let backup_error_id = backup_error_id.to_owned();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, backup, library, file, date, error FROM Backups_Errors WHERE id = ?")?;
            let row = query.query_row(
            [backup_error_id],Self::backup_error_from_row,
            ).optional()?;
            Ok(row)
           
        }).await?;
        Ok(row)
    }

    pub async fn get_backup_errors(&self, backup_id: &str) -> Result<Vec<BackupError>> {
        let backup_id = backup_id.to_owned();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, backup, library, file, date, error FROM Backups_Errors WHERE backup = ? ORDER BY date DESC")?;
            let rows = query.query_map(
            [backup_id],Self::backup_error_from_row,
            )?;
            let files:Vec<BackupError> = rows.collect::<std::result::Result<Vec<BackupError>, rusqlite::Error>>()?; 
            Ok(files)
        }).await?;
        Ok(row)
    }

    pub async fn add_backup_error(&self, error: BackupError) -> Result<()> {
        self.server_store.call( move |conn| { 
            conn.execute("INSERT INTO Backups_Errors (id, backup, library, file, date, error)
            VALUES (?, ?, ? ,?, ?, ?)", params![
                error.id,
                error.backup,
                error.library,
                error.file,
                error.date,
                error.error,
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_backup_error(&self, id: String) -> Result<()> {
        self.server_store.call( move |conn| { 
            conn.execute("DELETE FROM Backups_Errors WHERE id = ?", &[&id])?;
            Ok(())
        }).await?;
        Ok(())
    }

}
