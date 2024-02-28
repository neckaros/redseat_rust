use rusqlite::{params, params_from_iter, OptionalExtension, ToSql};

use crate::{domain::backup::Backup, model::{backups::BackupForUpdate, store::SqliteStore}};
use super::Result;



impl SqliteStore {
  
    
    pub async fn get_backups(&self) -> Result<Vec<Backup>> {
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, source, credentials, library, path, schedule, filter, last, password, size  FROM Backups")?;
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
            let mut query = conn.prepare("SELECT id, source, credentials, library, path, schedule, filter, last, password, size  FROM Backups WHERE id = ?")?;
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
            super::add_for_sql_update(update.credentials, "credentials", &mut columns, &mut values);
            super::add_for_sql_update(update.library, "library", &mut columns, &mut values);
            super::add_for_sql_update(update.path, "path", &mut columns, &mut values);
            super::add_for_sql_update(update.schedule, "schedule", &mut columns, &mut values);
            super::add_for_sql_update(update.filter, "filter", &mut columns, &mut values);
            super::add_for_sql_update(update.last, "last", &mut columns, &mut values);
            super::add_for_sql_update(update.password, "password", &mut columns, &mut values);
            super::add_for_sql_update(update.size, "size", &mut columns, &mut values);


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

            conn.execute("INSERT INTO Backups (id, source, credentials, library, path, schedule, filter, last, password, size)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?)", params![
                backup.id,
                backup.source,
                backup.credentials,
                backup.library,
                backup.path,
                backup.schedule,
                backup.filter,
                backup.last,
                backup.password,
                backup.size
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
}