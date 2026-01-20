use rs_plugin_common_interfaces::domain::rs_ids::RsIds;
use rusqlite::{params, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, OptionalExtension, Row, ToSql};
use serde::{Deserialize, Serialize};

use rs_plugin_common_interfaces::MediaType;

use crate::{domain::{library::LibraryLimits, view_progress::{ViewProgress, ViewProgressForAdd}, watched::{Watched, WatchedForAdd}}, model::{store::{from_comma_separated, sql::library, SqliteStore}, users::{HistoryQuery, ServerUser, ServerUserForUpdate, ServerUserLibrariesRights, ServerUserLibrariesRightsWithUser, ServerUserPreferences, UploadKey, UserRole, ViewProgressQuery}}, plugins::sources::error::SourcesError};

use super::{super::Error, deserialize_from_row, OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder, SqlOrder, SqlWhereType};
use super::Result;


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct WatchedQuery {
    
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub after: Option<i64>,
}

// region:    --- Library Settings

impl FromSql for ServerUserPreferences {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {

            let r = serde_json::from_str::<ServerUserPreferences>(&as_string).map_err(|_| FromSqlError::InvalidType)?;

            Ok(r)
        })
    }
}

impl ToSql for ServerUserPreferences {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let r = serde_json::to_string(&self).map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        Ok(ToSqlOutput::from(r))
    }
}
// endregion:    --- 


/// User object store
impl SqliteStore {
    // region:    --- Users
    pub async fn get_user(&self, user_id: &str) -> Result<ServerUser> {
        let user_id = user_id.to_string();
        let error_user_id = user_id.to_string();
            let user = self.server_store.call( move |conn| { 
                let mut user = conn.query_row(
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
                )?;
                
                
                    let mut stmt = conn.prepare("SELECT lur.library_ref, lur.roles, lib.name, lib.type, lur.limits FROM Libraries_Users_Rights as lur LEFT JOIN Libraries as lib ON lur.library_ref = lib.id WHERE user_ref = ?1")?;
                    
                    let person_iter = stmt.query_map([&user_id], |row| {
                        let mut limits: LibraryLimits = deserialize_from_row(row, 4)?;
                        limits.user_id = Some(user_id.clone());
                        Ok(ServerUserLibrariesRights {
                            id: row.get(0)?,
                            name: row.get(2)?,
                            kind: row.get(3)?,
                            roles: from_comma_separated(row.get(1)?),
                            limits
                        })
                    })?;
                    user.libraries = person_iter.flat_map(|e| e.ok()).collect::<Vec<ServerUserLibrariesRights>>();
                    Ok(user)
   

                

                
        }).await.map_err(|err| {
            match err {
                tokio_rusqlite::Error::Rusqlite(rusqlite::Error::QueryReturnedNoRows) => Error::UserNotFound(error_user_id),
               _ => err.into()
            }
        })?;
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

            let mut query = conn.prepare("SELECT lur.library_ref, lur.roles, lib.name, lib.type, lur.user_ref, lur.limits FROM Libraries_Users_Rights as lur LEFT JOIN Libraries as lib ON lur.library_ref = lib.id")?;
            let rights = query.query_map([],
            |row| {
                let user_id: String = row.get(4)?;
                let mut limits: LibraryLimits = deserialize_from_row(row, 5)?;
                limits.user_id = Some(user_id.clone());

                Ok(ServerUserLibrariesRightsWithUser {
                    id: row.get(0)?,
                    user_id,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    roles: from_comma_separated(row.get(1)?),
                    limits
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
                    roles: l.roles.clone(),
                    limits: l.limits.clone()
                }).collect::<Vec<ServerUserLibrariesRights>>();
            }
            Ok(users)
        }).await?;
        Ok(row)
    }


    pub async fn add_user(&self, user: ServerUser) -> Result<()> {
        self.server_store.call( move |conn| { 

            conn.execute("INSERT INTO Users (id, name, role, preferences)
            VALUES (?, ?, ?, ?)", params![
                user.id,
                user.name,
                user.role,
                user.preferences
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn update_user(&self, connected_user: &ServerUser, update_user: ServerUserForUpdate) -> Result<()> {
            if connected_user.id != update_user.id && connected_user.role != UserRole::Admin {
                return Err(Error::UserUpdateNotAuthorized { user: connected_user.clone(), update_user })
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
                        (serde_json::to_string(&preferences).map_err(|err| tokio_rusqlite::Error::Other(Box::new(err)))?, &update_user.id),
                    )?;
                }
                Ok(())
    
            }).await?;
            
            Ok(())
        }
    // endregion:    --- Users

}

///Upload key store
impl SqliteStore {
    
    pub async fn get_upload_key(&self, key: String) -> Result<UploadKey> {
        let keyc = key.clone();
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, library_ref, expiry, tags  FROM uploadkeys where id = ?")?;

            let rows = query.query_map(
            params![key], Self::row_to_uploadkey,
            )?;
            let backups:Vec<UploadKey> = rows.collect::<std::result::Result<Vec<UploadKey>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        let uploadkey = row.first().ok_or(SourcesError::UnableToFindUploadKey("store".to_string() ,keyc, "get_upload_key".to_string()))?;
        Ok(uploadkey.clone())
    }

    fn row_to_uploadkey(row: &Row) -> rusqlite::Result<UploadKey> {
        Ok(UploadKey {
            id: row.get(0)?,
            library: row.get(1)?,
            expiry: row.get(2)?,
            tags: row.get(3)?,
            
        })
    }
}

/// Watched store
impl SqliteStore {
    fn row_to_watched(row: &Row) -> rusqlite::Result<Watched> {
        Ok(Watched {
            kind: row.get(0)?,
            id: row.get(1)?,
            user_ref: row.get(2)?,
            date: row.get(3)?,
            modified: row.get(4)?,
            
        })
    }

    pub async fn get_all_watched(&self) -> Result<Vec<Watched>> {
        let row = self.server_store.call( move |conn| { 
            
            let mut query = conn.prepare(&format!("SELECT type, id, user_ref, date, modified  FROM Watched"))?;

            let rows = query.query_map(
            params![], Self::row_to_watched,
            )?;
            let backups:Vec<Watched> = rows.collect::<std::result::Result<Vec<Watched>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }

    pub async fn get_watched(&self, query: HistoryQuery, user_id: String, other_users: Vec<String>) -> Result<Vec<Watched>> {
        let row = self.server_store.call( move |conn| {
            let mut where_query = RsQueryBuilder::new();

            // Filter out deleted (date = 0) items unless include_deleted is true
            if !query.include_deleted {
                where_query.add_where(SqlWhereType::After("date".to_owned(), Box::new(0i64)));
            }

            if let Some(q) = query.after {
                where_query.add_where(SqlWhereType::After("modified".to_owned(), Box::new(q)));
            }
            if !query.types.is_empty() {
                let mut types = vec![];
                for kind in query.types {
                    types.push(SqlWhereType::Equal("type".to_owned(), Box::new(kind)));
                }
                where_query.add_where(SqlWhereType::Or(types));
            }
            if other_users.len() > 0 {
                let mut types = vec![];
                 types.push(SqlWhereType::Equal("user_ref".to_owned(), Box::new(user_id)));
                 for other_user in other_users {
                    types.push(SqlWhereType::Equal("user_ref".to_owned(), Box::new(other_user)));
                 }
                 
                where_query.add_where(SqlWhereType::Or(types));
            } else {
                where_query.add_where(SqlWhereType::Equal("user_ref".to_owned(), Box::new(user_id)));
            }
            
            
            if let Some(ids) = query.id {
                let ids: Vec<String> = ids.into();
                let ids = ids.into_iter().map(|f| Box::new(f) as Box<dyn ToSql>).collect();
                where_query.add_where(SqlWhereType::In("id".to_owned(), ids));
            }

            where_query.add_oder(OrderBuilder::new(query.sort.to_string(), query.order));

            let mut query = conn.prepare(&format!("SELECT type, id, user_ref, date, modified  FROM Watched {}{}", where_query.format(), where_query.format_order()))?;

            let rows = query.query_map(
            where_query.values(), Self::row_to_watched,
            )?;
            let backups:Vec<Watched> = rows.collect::<std::result::Result<Vec<Watched>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }


    pub async fn add_watched(&self, watched: WatchedForAdd, user_id: String) -> Result<()> {
        self.server_store.call( move |conn| {

            conn.execute("INSERT OR REPLACE INTO Watched (type, id, user_ref, date)
            VALUES (?, ? ,?, ?)", params![
                watched.kind,
                watched.id,
                user_id,
                watched.date
            ])?;

            conn.execute("DELETE FROM progress where type = ? and id = ? and user_ref = ?", params![
                watched.kind,
                watched.id,
                user_id,
            ])?;

            Ok(())
        }).await?;
        Ok(())
    }

    /// Marks watched entries as deleted by setting date=0
    /// This allows clients to sync deletions via the history API with ?includeDeleted=true
    /// Returns the list of IDs that were actually marked as deleted
    pub async fn delete_watched(&self, kind: MediaType, ids: Vec<String>, user_ref: String) -> Result<Vec<String>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let deleted_ids = self.server_store.call(move |conn| {
            let mut deleted_ids = Vec::new();
            for id in ids {
                let rows_affected = conn.execute(
                    "UPDATE Watched SET date = 0, modified = ? WHERE type = ? AND id = ? AND user_ref = ? AND date != 0",
                    params![now, kind, &id, &user_ref]
                )?;
                if rows_affected > 0 {
                    deleted_ids.push(id);
                }
            }
            Ok(deleted_ids)
        }).await?;
        Ok(deleted_ids)
    }
}


// Copy watched
/*BEGIN TRANSACTION;
INSERT INTO Watched (type, id, user_ref, date, modified)
SELECT type, id, '7YDaetkfMjcbf9cNlJLZsVtwbNu2', date, modified
FROM Watched
WHERE user_ref = 'VMBGvIefEVhfRoa7Fu7cXXcfdux1';
COMMIT; */

/// Progress Store
impl SqliteStore {
    
    fn row_to_view_progress(row: &Row) -> rusqlite::Result<ViewProgress> {
        Ok(ViewProgress {
            kind: row.get(0)?,
            id: row.get(1)?,
            user_ref: row.get(2)?,
            progress: row.get(3)?,
            parent: row.get(4)?,
            modified: row.get(5)?,
            
        })
    }

    
    pub async fn get_all_view_progress(&self, query: HistoryQuery, user_id: String) -> Result<Vec<ViewProgress>> {
        let row = self.server_store.call( move |conn| { 
            let mut where_query = RsQueryBuilder::new();
            if let Some(q) = query.after {
                where_query.add_where(SqlWhereType::After("modified".to_owned(), Box::new(q)));
            }
            if !query.types.is_empty() {
                let mut types = vec![];
                for kind in query.types {
                    types.push(SqlWhereType::Equal("type".to_owned(), Box::new(kind)));
                }
                where_query.add_where(SqlWhereType::Or(types));
            }
            where_query.add_where(SqlWhereType::Equal("user_ref".to_owned(), Box::new(user_id)));
            if let Some(ids) = query.id {
                let ids: Vec<String> = ids.into();
                let ids = ids.into_iter().map(|f| Box::new(f) as Box<dyn ToSql>).collect();
                where_query.add_where(SqlWhereType::In("id".to_owned(), ids));
            }

            where_query.add_oder(OrderBuilder::new(query.sort.to_string(), query.order));

            let mut query = conn.prepare(&format!("SELECT type, id, user_ref, progress, parent, modified  FROM progress {}{}", where_query.format(), where_query.format_order()))?;

            let rows = query.query_map(
            where_query.values(), Self::row_to_view_progress,
            )?;
            let backups:Vec<ViewProgress> = rows.collect::<std::result::Result<Vec<ViewProgress>, rusqlite::Error>>()?; 
            Ok(backups)
        }).await?;
        Ok(row)
    }


    pub async fn get_view_progess(&self, ids: RsIds, user_id: String) -> Result<Option<ViewProgress>> {
        let row = self.server_store.call( move |conn| { 
            let mut builder_query = RsQueryBuilder::new();

            builder_query.add_where(SqlWhereType::Equal("user_ref".to_owned(), Box::new(user_id)));
 
            let ids: Vec<String> = ids.into();
            let ids = ids.into_iter().map(|f| Box::new(f) as Box<dyn ToSql>).collect();
            builder_query.add_where(SqlWhereType::In("id".to_owned(), ids));

            let mut query = conn.prepare(&format!("SELECT type, id, user_ref, progress, parent, modified  FROM progress {}", builder_query.format()))?;
   
            let row = query.query_row(
                builder_query.values(), Self::row_to_view_progress,
            ).optional()?;
            Ok(row)
        }).await?;
        Ok(row)
    }

    pub async fn add_view_progress(&self, progress: ViewProgressForAdd, user_ref: String) -> Result<()> {
        self.server_store.call( move |conn| { 

            conn.execute("INSERT OR REPLACE INTO progress (type, id, user_ref, progress, parent)
            VALUES (?, ?, ? ,?,?)", params![
                progress.kind,
                progress.id,
                user_ref,
                progress.progress,
                progress.parent
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }



}
    
    