use std::collections::HashMap;
use std::str::FromStr;

use tokio_rusqlite::Connection;


use crate::model::store::sql::{migrate_database};
use crate::server::get_server_file_path_array;
use crate::tools::log::{log_info, LogServiceType};

use self::sql::libraries;
use self::sql::library::SqliteLibraryStore;

use super::error::{Result, Error};


mod sql;


pub struct SqliteStore {
	server_store: Connection,
    libraries_stores: HashMap<String, SqliteLibraryStore>
}

// Constructor
impl SqliteStore {
	pub async fn new() -> Result<Self> {
        let server_db_path = get_server_file_path_array(&mut vec![&"dbs", &"database.db"]).await.map_err(|_| Error::CannotOpenDatabase)?;
        let connection = Connection::open(server_db_path).await?;
        
        let version = migrate_database(&connection).await?;

    
        log_info(LogServiceType::Database, format!("Current Database version: {}", version));
        let mut new = Self {
			server_store: connection,
            libraries_stores: HashMap::new()
		};

        let libraries = new.get_libraries().await?;
        for library in libraries {
            log_info(LogServiceType::Database, format!("Initiatin database: {}", &library.name));
            let server_db_path = get_server_file_path_array(&mut vec![&"dbs", &format!("{}.db", &library.id)]).await.map_err(|_| Error::CannotOpenDatabase)?;
            let library_connection = Connection::open(server_db_path).await?;
            let library_store = SqliteLibraryStore::new(library_connection).await?;
            new.libraries_stores.insert(library.id.to_string(), library_store);
        }

		Ok(new)
	}

    pub fn get_library_store(&self, library_id: &str) -> Option<&SqliteLibraryStore> {
        self.libraries_stores.get(library_id)
    }

}

fn from_comma_separated<T: FromStr>(text: String) -> Vec<T> {
    text.split(",").map(|s| s.trim()).filter_map(|s| T::from_str(s).ok()).collect::<Vec<T>>()
}
fn from_comma_separated_optional<T: FromStr>(text: Option<String>) -> Option<Vec<T>> {
    if let Some(text) = text {
        Some(text.split(",").map(|s| s.trim()).filter_map(|s| T::from_str(s).ok()).collect::<Vec<T>>())
    } else {
        None
    }
}
fn to_comma_separated<T: ToString>(elements: Vec<T>) -> String {
    elements.into_iter().map(|e| e.to_string()).collect::<Vec<String>>().join(",")
}
fn to_comma_separated_optional<T: ToString>(elements: Option<Vec<T>>) -> Option<String> {
    if let Some(elements) = elements {
        Some(elements.into_iter().map(|e| e.to_string()).collect::<Vec<String>>().join(","))
    } else {
        None
    }
}

