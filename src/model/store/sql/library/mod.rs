use rusqlite::params;
use tokio_rusqlite::Connection;
use crate::{model::{tags::TagQuery, ModelController}, tools::log::{log_info, LogServiceType}};

use super::Result;

pub mod tags;
pub mod people;
pub mod series;
pub mod episodes;

pub struct SqliteLibraryStore {
	connection: Connection,
}

// Constructor
impl SqliteLibraryStore {
	pub async fn new(connection: Connection) -> Result<Self> {
        let new = Self {
            connection
        };
        new.migrate().await?;
        Ok(new)
    }


    pub async fn migrate(&self) -> Result<usize> {
        let (initial_version, version) = self.connection.call( |conn| {
            let mut version = conn.query_row(
                "SELECT user_version FROM pragma_user_version;",
                [],
                |row| {
                    let version: usize = row.get(0)?;
                    Ok(version)
                })?;
                let initial_version = version.clone();
                if version < 28 {
                    let initial = String::from_utf8_lossy(include_bytes!("001 - INITIAL.sql"));
                    conn.execute_batch(&initial)?;
                    version = 28;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(LogServiceType::Database, format!("Update Library Database to version: {}", version));
                }
                if version < 29 {
                    let initial = String::from_utf8_lossy(include_bytes!("029 - TAGPATH.sql"));
                    conn.execute_batch(&initial)?;
                    version = 29;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(LogServiceType::Database, format!("Update Library Database to version: {}", version));                   
                }
                
                if version < 30 {
                    let initial = String::from_utf8_lossy(include_bytes!("030 - INSERT TRIGGERS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 30;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(LogServiceType::Database, format!("Update Library Database to version: {}", version));                   
                }
                
                Ok((initial_version, version))
        }).await?;

        if initial_version < 29 {
            log_info(LogServiceType::Database, format!("Update Library Database adding tag paths"));                   
            let tags = self.get_tags(TagQuery::new_empty()).await?;
            let tags = ModelController::fill_tags_paths(None, "/", &tags);
            self.connection.call( |conn| {
                for tag in tags {
                    conn.execute("UPDATE tags SET path = ? WHERE id = ?", params![tag.path, tag.id])?;
                }
                Ok(())
            }).await?;
            log_info(LogServiceType::Database, format!("Update Library Database adding tag paths / Done"));                   
        }

        Ok(version)
    } 
}