use tokio_rusqlite::Connection;


use crate::server::get_server_file_path;

use super::error::{Result, Error};

use super::users::{ServerUser, ServerUserPreferences};



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
                "SELECT id, name, role, preferences  FROM Users WHERE id = ?1",
                [&user_id],
                |row| {
                    let preferences_string: String = row.get(3)?;
                    let preferences: ServerUserPreferences = serde_json::from_str(&preferences_string).unwrap();
                    Ok(ServerUser {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        role:  row.get(2)?,
                        preferences
                    })
                },
                )?;

                Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn get_users(&self) -> Result<Vec<ServerUser>> {
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, name, role, preferences  FROM Users")?;
            let rows = query.query_map(
            [],
            |row| {
                let preferences_string: String = row.get(3)?;
                let preferences: ServerUserPreferences = serde_json::from_str(&preferences_string).unwrap();
                Ok(ServerUser {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    role:  row.get(2)?,
                    preferences
                })
            },
            )?;
            let mut vector: Vec<ServerUser> = [].to_vec();
            for person in rows {
                let ok: ServerUser = person?;
                vector.push(ok)
            }

            Ok(vector)
        }).await?;
        Ok(row)
    }
}