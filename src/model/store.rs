use tokio_rusqlite::Connection;

use crate::server::get_server_file_path;

use super::error::{Result, Error};

use super::ServerUser;



pub struct SqliteStore {
	server_store: Connection 
}


// Constructor
impl SqliteStore {
	pub async fn new() -> Result<Self> {
        let server_db_path = get_server_file_path("database.db").await.map_err(|_| Error::CannotOpenDatabase)?;
        let connection = Connection::open(server_db_path).await?;
		Ok(Self {
			server_store: connection
		})
	}
}

impl SqliteStore {

    pub async fn get_user(&self, user_id: &str) -> Result<ServerUser> {
        let user_id = user_id.to_string();
            let row = self.server_store.call( move |conn| { 
                let row = conn.query_row(
                "SELECT name FROM Users WHERE id = ?1",
                [&user_id],
                |r| {
                    let name: String = r.get(0)?;

                    Ok(ServerUser {
                        user_id: user_id.clone(),
                        name 
                    })
                },
                )?;

                Ok(row)
        }).await?;
        Ok(row)
    }
}