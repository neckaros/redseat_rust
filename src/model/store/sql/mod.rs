pub mod libraries;
pub mod users;
pub mod credentials;
pub mod backups;
pub mod library;

use rusqlite::ToSql;
use tokio_rusqlite::Connection;

use super::{Result, SqliteStore};


pub async fn migrate_database(connection: &Connection) -> Result<usize> {
    let version = connection.call( |conn| {
        let version = conn.query_row(
            "SELECT user_version FROM pragma_user_version;",
            [],
            |row| {
                let version: usize = row.get(0)?;
                Ok(version)
            })?;

            if version < 2 {
                let initial = String::from_utf8_lossy(include_bytes!("001 - INITIAL.sql"));
                conn.execute_batch(&initial)?;
                
                conn.pragma_update(None, "user_version", 2)?;
                println!("Update SQL to verison 2")
            }
            
            Ok(version)
    }).await?;

    Ok(version)
} 

pub fn add_for_sql_update<'a, T: ToSql + 'a,>(optional: Option<T>, name: &str, columns: &mut Vec<String>, values: &mut Vec<Box<dyn ToSql + 'a>>) {
    if let Some(value) = optional {
        let r = format!("{} = ?", name.to_string());
        columns.push(r);
        values.push(Box::new(value));
    } 
}

