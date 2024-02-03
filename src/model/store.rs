use std::str::FromStr;

use tokio_rusqlite::Connection;


use crate::server::get_server_file_path;

use super::error::{Result, Error};

use super::libraries::ServerLibrary;
use super::users::{ServerUser, ServerUserForUpdate, ServerUserLibrariesRights, ServerUserLibrariesRightsWithUser, ServerUserPreferences, UserRole};



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

fn from_comma_separated<T: FromStr>(text: String) -> Vec<T> {
    text.split(",").map(|s| s.trim()).filter_map(|s| T::from_str(s).ok()).collect::<Vec<T>>()
}
fn to_comma_separated<T: ToString>(elements: Vec<T>) -> String {
    elements.into_iter().map(|e| e.to_string()).collect::<Vec<String>>().join(",")
}


impl SqliteStore {
// region:    --- Users
    pub async fn get_user(&self, user_id: &str) -> Result<ServerUser> {
        let user_id = user_id.to_string();
            let user = self.server_store.call( move |conn| { 
                let user = conn.query_row(
                "SELECT id, name, role, preferences  FROM Users WHERE id = ?1",
                [&user_id],
                |row| {
                    let preferences_string: String = row.get(3)?;
                    let preferences: ServerUserPreferences = serde_json::from_str(&preferences_string).unwrap();
                    Ok(ServerUser {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        role:  row.get(2)?,
                        preferences,
                        libraries: vec![]
                    })
                },
                ).and_then(|mut el| {
                    let mut stmt = conn.prepare("SELECT lur.library_ref, lur.roles, lib.name, lib.type FROM Libraries_Users_Rights as lur LEFT JOIN Libraries as lib ON lur.library_ref = lib.id WHERE user_ref = ?1")?;
                    let person_iter = stmt.query_map([&user_id], |row| {
                        Ok(ServerUserLibrariesRights {
                            id: row.get(0)?,
                            name: row.get(2)?,
                            kind: row.get(3)?,
                            roles: from_comma_separated(row.get(1)?)
                        })
                    })?;
                    el.libraries = person_iter.map(|e| e.ok()).filter_map(|e| e).collect::<Vec<ServerUserLibrariesRights>>();
                    Ok(el)
                })?;

               

                Ok(user)
        }).await?;
        Ok(user)
    }

    pub async fn get_users(&self) -> Result<Vec<ServerUser>> {
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, name, role, preferences  FROM Users")?;
            let users = query.query_map([],
            |row| {
                let preferences_string: String = row.get(3)?;
                let preferences: ServerUserPreferences = serde_json::from_str(&preferences_string).unwrap();
                Ok(ServerUser {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    role:  row.get(2)?,
                    preferences,
                    libraries: vec![]
                })
            },
            )?;

            let mut query = conn.prepare("SELECT lur.library_ref, lur.roles, lib.name, lib.type, lur.user_ref FROM Libraries_Users_Rights as lur LEFT JOIN Libraries as lib ON lur.library_ref = lib.id")?;
            let rights = query.query_map([],
            |row| {
                Ok(ServerUserLibrariesRightsWithUser {
                    id: row.get(0)?,
                    user_id: row.get(4)?,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    roles: from_comma_separated(row.get(1)?)
                })
            },
            )?;

            let mut users: Vec<ServerUser> = users.collect::<std::result::Result<Vec<ServerUser>, rusqlite::Error>>()?;

    
            let rights: Vec<ServerUserLibrariesRightsWithUser> = rights.collect::<std::result::Result<Vec<ServerUserLibrariesRightsWithUser>, rusqlite::Error>>()?;

            for person in &mut users {
                person.libraries = rights.iter().filter(|l| l.user_id == person.id).map(|l| ServerUserLibrariesRights {
                    id: l.id.clone(),
                    name: l.name.clone(),
                    kind: l.kind.clone(),
                    roles: l.roles.clone()
                }).collect::<Vec<ServerUserLibrariesRights>>();
            }
            Ok(users)
        }).await?;
        Ok(row)
    }

    pub async fn update_user(&self, connected_user: &ServerUser, update_user: ServerUserForUpdate) -> Result<()> {
        if connected_user.id != update_user.id && connected_user.role != UserRole::Admin {
            return Err(Error::UserUpdateNotAuthorized { user: connected_user.clone(), update_user: update_user })
        }

        if update_user.role.is_some() && connected_user.role != UserRole::Admin {
            return Err(Error::UserRoleUpdateNotAuthOnlyAdmin)
        }

        self.server_store.call( move |conn| { 
            if let Some(name) = update_user.name {
                conn.execute(
                    "UPDATE Users SET name = ?1 WHERE ID = ?2",
                    (name, &update_user.id),
                )?;
            }
            if let Some(role) = update_user.role {
                conn.execute(
                    "UPDATE Users SET role = ?1 WHERE ID = ?2",
                    (role, &update_user.id),
                )?;
            }

            if let Some(preferences) = update_user.preferences {
                conn.execute(
                    "UPDATE Users SET role = ?1 WHERE ID = ?2",
                    (serde_json::to_string(&preferences).or_else(|err| Err(tokio_rusqlite::Error::Other(Box::new(err))))?, &update_user.id),
                )?;
            }
            Ok(())

        }).await?;
        
        Ok(())
    }
// endregion:    --- Users


// region:    --- Libraries
pub async fn get_library(&self, user_id: &str) -> Result<ServerLibrary> {
    let user_id = user_id.to_string();
        let row = self.server_store.call( move |conn| { 
            let row = conn.query_row(
            "SELECT id, name, source, root, type, crypt, settings  FROM Libraries WHERE id = ?1",
            [&user_id],
            |row| {
                Ok(ServerLibrary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    source:  row.get(2)?,
                    root:  row.get(3)?,
                    kind:  row.get(4)?,
                    crypt:  row.get(5)?,
                    settings:  row.get(6)?,
                })
            },
            )?;

            Ok(row)
    }).await?;
    Ok(row)
}

pub async fn get_libraries(&self) -> Result<Vec<ServerLibrary>> {
    let row = self.server_store.call( move |conn| { 
        let mut query = conn.prepare("SELECT id, name, source, root, type, crypt, settings  FROM Libraries")?;
        let rows = query.query_map(
        [],
        |row| {
            Ok(ServerLibrary {
                id: row.get(0)?,
                name: row.get(1)?,
                source:  row.get(2)?,
                root:  row.get(3)?,
                kind:  row.get(4)?,
                crypt:  row.get(5)?,
                settings:  row.get(6)?,
            })
        },
        )?;
        let libraries:Vec<ServerLibrary> = rows.collect::<std::result::Result<Vec<ServerLibrary>, rusqlite::Error>>()?; 

        Ok(libraries)
    }).await?;
    Ok(row)
}

// endregion:    --- Libraries

}