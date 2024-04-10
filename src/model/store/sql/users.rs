use rusqlite::{params, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, Row, ToSql};
use serde::{Deserialize, Serialize};

use crate::{domain::watched::Watched, model::{store::{from_comma_separated, sql::library, SqliteStore}, users::{HistoryQuery, ServerUser, ServerUserForUpdate, ServerUserLibrariesRights, ServerUserLibrariesRightsWithUser, ServerUserPreferences, UploadKey, UserRole}}};

use super::{super::Error, OrderBuilder, QueryBuilder, QueryWhereType, SqlOrder};
use super::Result;


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct WatchedQuery {
    
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub after: Option<u64>,
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
                    el.libraries = person_iter.flat_map(|e| e.ok()).collect::<Vec<ServerUserLibrariesRights>>();
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



    fn row_to_watched(row: &Row) -> rusqlite::Result<Watched> {
        Ok(Watched {
            kind: row.get(0)?,
            source: row.get(1)?,
            id: row.get(2)?,
            user_ref: row.get(3)?,
            date: row.get(4)?,
            modified: row.get(5)?,
            
        })
    }
    fn row_to_uploadkey(row: &Row) -> rusqlite::Result<UploadKey> {
        Ok(UploadKey {
            id: row.get(0)?,
            library: row.get(1)?,
            expiry: row.get(2)?,
            tags: row.get(3)?,
            
        })
    }

    pub async fn get_watched(&self, query: HistoryQuery) -> Result<Vec<Watched>> {
        let row = self.server_store.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            if let Some(q) = &query.after {
                where_query.add_where(QueryWhereType::After("modified", q));
            }
            if query.types.len() > 0 {
                let mut types = vec![];
                for kind in &query.types {
                    types.push(QueryWhereType::Equal("type", kind));
                }
                where_query.add_where(QueryWhereType::Or(types));
            }

            where_query.add_oder(OrderBuilder::new(query.sort.to_string(), query.order));
            
            let source_name = "trakt".to_string();
            let trakt = query.id.and_then(|i| i.trakt);
            if let Some(trakt) = &trakt {
                let mut list = vec![];
                list.push(QueryWhereType::Equal("source", &source_name ));
                list.push(QueryWhereType::Equal("id", trakt));
                where_query.add_where(QueryWhereType::And(list));
                
            }
            let mut query = conn.prepare(&format!("SELECT type, source, id, user_ref, date, modified  FROM Watched {}{}", where_query.format(), where_query.format_order()))?;

            let rows = query.query_map(
            where_query.values(), Self::row_to_watched,
            )?;
            let backups:Vec<Watched> = rows.collect::<std::result::Result<Vec<Watched>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }

    pub async fn get_upload_key(&self, key: String) -> Result<UploadKey> {
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, library_ref, expiry, tags  FROM uploadkeys where id = ?")?;

            let rows = query.query_map(
            params![key], Self::row_to_uploadkey,
            )?;
            let backups:Vec<UploadKey> = rows.collect::<std::result::Result<Vec<UploadKey>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        let uploadkey = row.get(0).ok_or(Error::NotFound)?;
        Ok(uploadkey.clone())
    }
}
    
    