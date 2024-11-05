use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use tokio_rusqlite::Connection;


use crate::error::RsResult;
use crate::model::store::sql::migrate_database;
use crate::server::get_server_file_path_array;
use crate::tools::log::{log_info, LogServiceType};

use self::sql::library::SqliteLibraryStore;

use super::error::{Result, Error};


pub mod sql;


pub struct SqliteStore {
	server_store: Connection,
    libraries_stores: RwLock<HashMap<String, Arc<SqliteLibraryStore>>>
}

// Constructor
impl SqliteStore {
	pub async fn new() -> Result<Self> {
        let server_db_path = get_server_file_path_array(vec![&"dbs", &"database.db"]).await.map_err(|_| Error::CannotOpenDatabase)?;
        let connection = Connection::open(server_db_path).await?;
        
        let version = migrate_database(&connection).await?;

    
        log_info(LogServiceType::Database, format!("Current Database version: {}", version));
        let mut new = Self {
			server_store: connection,
            libraries_stores: RwLock::new(HashMap::new())
		};

        let libraries = new.get_libraries().await?;
        for library in libraries {
            log_info(LogServiceType::Database, format!("Loading database: {}", &library.name));
            let server_db_path = get_server_file_path_array(vec![&"dbs", &format!("db-{}.db", &library.id)]).await.map_err(|_| Error::CannotOpenDatabase)?;
            let library_connection = Connection::open(server_db_path).await?;
            let library_store = SqliteLibraryStore::new(library_connection).await?;
            new.libraries_stores.write().map_err(|e| Error::ServiceError("Failed acquiring lock for libraries stores".to_string(), Some(e.to_string())))?.insert(library.id.to_string(), Arc::new(library_store));
        }

		Ok(new)
	}

    pub fn get_library_store(&self, library_id: &str) -> Option<Arc<SqliteLibraryStore>> {
        let value = self.libraries_stores.read().ok()?;
        value.get(library_id).cloned()
    }

    
    pub async fn add_library_to_store(&self, library_id: &str) -> RsResult<()> {
        let library = self.get_library(library_id).await?.ok_or(Error::LibraryNotFound(library_id.to_string()))?;
        self.libraries_stores.write().map_err(|e| Error::ServiceError("Failed acquiring lock for libraries stores".to_string(), Some(e.to_string())))?.remove(library_id);
        log_info(LogServiceType::Database, format!("Loading database: {}", &library.name));
        let server_db_path = get_server_file_path_array(vec![&"dbs", &format!("db-{}.db", &library.id)]).await.map_err(|_| Error::CannotOpenDatabase)?;
        let library_connection = Connection::open(server_db_path).await.map_err(|e| Error::ServiceError("Unable to open database".to_string(), Some(e.to_string())))?;
        let library_store = SqliteLibraryStore::new(library_connection).await?;
        self.libraries_stores.write().map_err(|e| Error::ServiceError("Failed acquiring lock for libraries stores".to_string(), Some(e.to_string())))?.insert(library.id.to_string(), Arc::new(library_store));
        Ok(())
    }

}

fn from_separated<T: FromStr>(text: String, separator: &str) -> Vec<T> {
    if text == "" {
        return vec![];
    }
    text.split(separator).map(|s| s.trim()).filter_map(|s| T::from_str(s).ok()).collect::<Vec<T>>()
}
fn to_separated<T: ToString>(elements: Vec<T>, separator: &str) -> String {
    if elements.len() == 0 {
        return "".into();
    }
    elements.into_iter().map(|e| e.to_string()).collect::<Vec<String>>().join(separator)
}

fn from_comma_separated<T: FromStr>(text: String) -> Vec<T> {
    from_separated(text, ",")
}
fn from_comma_separated_optional<T: FromStr>(text: Option<String>) -> Option<Vec<T>> {
    if let Some(text) = text {
        Some(from_separated(text, ","))
    } else {
        None
    }
}


fn to_comma_separated<T: ToString>(elements: Vec<T>) -> String {
    to_separated(elements, ",")
}
fn to_comma_separated_optional<T: ToString>(elements: Option<Vec<T>>) -> Option<String> {
    if let Some(elements) = elements {
        Some(to_separated(elements, ","))
    } else {
        None
    }
}



fn from_pipe_separated<T: FromStr>(text: String) -> Vec<T> {
    from_separated(text, "|")
}
fn from_pipe_separated_optional<T: FromStr>(text: Option<String>) -> Option<Vec<T>> {
    if let Some(text) = text {
        Some(from_separated(text, "|"))
    } else {
        None
    }
}
fn to_pipe_separated<T: ToString>(elements: Vec<T>) -> String {
    to_separated(elements, "|")
}
fn to_pipe_separated_optional<T: ToString>(elements: Option<Vec<T>>) -> Option<String> {
    if let Some(elements) = elements {
        Some(to_separated(elements, "|"))
    } else {
        None
    }
}
