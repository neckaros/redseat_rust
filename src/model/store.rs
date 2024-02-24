use std::str::FromStr;

use tokio_rusqlite::Connection;


use crate::model::store::sql::migrate_database;
use crate::server::get_server_file_path;
use crate::tools::log::{log_info, LogServiceType};

use super::error::{Result, Error};


mod sql;


pub struct SqliteStore {
	server_store: Connection 
}


// Constructor
impl SqliteStore {
	pub async fn new() -> Result<Self> {
        let server_db_path = get_server_file_path("database.db").await.map_err(|_| Error::CannotOpenDatabase)?;
        let connection = Connection::open(server_db_path).await?;
        
        let version = migrate_database(&connection).await?;

    
        log_info(LogServiceType::Database, format!("Current Database version: {}", version));


		Ok(Self {
			server_store: connection
		})
	}
}

fn from_comma_separated<T: FromStr>(text: String) -> Vec<T> {
    text.split(",").map(|s| s.trim()).filter_map(|s| T::from_str(s).ok()).collect::<Vec<T>>()
}
fn to_comma_separated<T: ToString>(elements: Vec<T>) -> String {
    elements.into_iter().map(|e| e.to_string()).collect::<Vec<String>>().join(",")
}


