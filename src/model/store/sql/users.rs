use rs_plugin_common_interfaces::domain::rs_ids::RsIds;
use rusqlite::{
    params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    OptionalExtension, Row, ToSql,
};
use serde::{Deserialize, Serialize};

use rs_plugin_common_interfaces::MediaType;

use crate::{
    domain::{
        library::LibraryLimits,
        view_progress::{ViewProgress, ViewProgressForAdd},
        watched::{Watched, WatchedForAdd},
    },
    model::{
        store::{from_comma_separated, sql::library, SqliteStore},
        users::{
            HistoryQuery, ServerUser, ServerUserForUpdate, ServerUserLibrariesRights,
            ServerUserLibrariesRightsWithUser, ServerUserPreferences, UploadKey, UserRole,
            ViewProgressQuery,
        },
    },
    plugins::sources::error::SourcesError,
};

use super::Result;
use super::{
    super::Error, deserialize_from_row, OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder,
    SqlOrder, SqlWhereType,
};

#[derive(Debug, Clone)]
pub struct HistoryIdRewrite {
    pub kind: MediaType,
    pub old_id: String,
    pub new_id: String,
    pub user_ref: String,
}

#[derive(Debug, Clone)]
pub struct ProgressIdRewrite {
    pub kind: MediaType,
    pub old_id: String,
    pub new_id: String,
    pub new_parent: Option<String>,
    pub user_ref: String,
}

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
            let r = serde_json::from_str::<ServerUserPreferences>(&as_string)
                .map_err(|_| FromSqlError::InvalidType)?;

            Ok(r)
        })
    }
}

impl ToSql for ServerUserPreferences {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let r = serde_json::to_string(&self)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
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
        self.server_store
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO Users (id, name, role, preferences)
            VALUES (?, ?, ?, ?)",
                    params![user.id, user.name, user.role, user.preferences],
                )?;

                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn update_user(
        &self,
        connected_user: &ServerUser,
        update_user: ServerUserForUpdate,
    ) -> Result<()> {
        if connected_user.id != update_user.id && connected_user.role != UserRole::Admin {
            return Err(Error::UserUpdateNotAuthorized {
                user: connected_user.clone(),
                update_user,
            });
        }

        if update_user.role.is_some() && connected_user.role != UserRole::Admin {
            return Err(Error::UserRoleUpdateNotAuthOnlyAdmin);
        }

        self.server_store
            .call(move |conn| {
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
                        "UPDATE Users SET preferences = ?1 WHERE ID = ?2",
                        (
                            serde_json::to_string(&preferences)
                                .map_err(|err| tokio_rusqlite::Error::Other(Box::new(err)))?,
                            &update_user.id,
                        ),
                    )?;
                }
                Ok(())
            })
            .await?;

        Ok(())
    }
    // endregion:    --- Users
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::RwLock};

    use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, MediaType};
    use tokio_rusqlite::Connection;

    use super::SqliteStore;
    use crate::{
        domain::{view_progress::ViewProgressForAdd, watched::WatchedForAdd},
        model::{
            store::sql::{
                migrate_database,
                users::{HistoryIdRewrite, ProgressIdRewrite},
            },
            users::HistoryQuery,
        },
    };

    async fn progress_store() -> SqliteStore {
        let connection = Connection::open_in_memory().await.unwrap();
        migrate_database(&connection).await.unwrap();
        SqliteStore {
            server_store: connection,
            libraries_stores: RwLock::new(HashMap::new()),
        }
    }

    async fn add_progress(store: &SqliteStore, id: &str, user_ref: &str, progress: u64) {
        store
            .add_view_progress(
                ViewProgressForAdd {
                    kind: MediaType::Movie,
                    id: id.to_string(),
                    parent: None,
                    progress,
                },
                user_ref.to_string(),
            )
            .await
            .unwrap();
    }

    async fn set_progress_modified(store: &SqliteStore, id: &str, user_ref: &str, modified: u64) {
        let (id, user_ref) = (id.to_string(), user_ref.to_string());
        store
            .server_store
            .call(move |conn| {
                conn.execute(
                    "UPDATE progress SET modified = ? WHERE id = ? AND user_ref = ?",
                    rusqlite::params![modified, id, user_ref],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// (id, user_ref) -> (progress, modified)
    async fn progress_rows(store: &SqliteStore) -> HashMap<(String, String), (u64, u64)> {
        store
            .get_all_view_progress_rows()
            .await
            .unwrap()
            .into_iter()
            .map(|row| ((row.id, row.user_ref), (row.progress, row.modified)))
            .collect()
    }

    fn key(id: &str, user_ref: &str) -> (String, String) {
        (id.to_string(), user_ref.to_string())
    }

    #[tokio::test]
    async fn progress_time_is_per_user_and_changes_with_the_position() {
        let store = progress_store().await;
        add_progress(&store, "imdb:tt1", "user-a", 1000).await;
        set_progress_modified(&store, "imdb:tt1", "user-a", 5).await;

        // Another user's playback leaves user A's time alone.
        add_progress(&store, "imdb:tt1", "user-b", 2000).await;
        let rows = progress_rows(&store).await;
        assert_eq!(rows[&key("imdb:tt1", "user-a")], (1000, 5));
        assert!(rows[&key("imdb:tt1", "user-b")].1 > 5);

        // Updating the position stamps it; other updates don't.
        store
            .server_store
            .call(|conn| {
                conn.execute(
                    "UPDATE progress SET parent = 'x' WHERE user_ref = 'user-a'",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(progress_rows(&store).await[&key("imdb:tt1", "user-a")].1, 5);
        store
            .server_store
            .call(|conn| {
                conn.execute(
                    "UPDATE progress SET progress = 3000 WHERE user_ref = 'user-a'",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(progress_rows(&store).await[&key("imdb:tt1", "user-a")].1 > 5);
    }

    #[tokio::test]
    async fn saving_progress_returns_the_stamped_row() {
        let store = progress_store().await;
        let saved = store
            .add_view_progress(
                ViewProgressForAdd {
                    kind: MediaType::Episode,
                    id: "episode:imdb/tt1/1/2".to_string(),
                    parent: Some("serie:imdb/tt1".to_string()),
                    progress: 1500,
                },
                "user-a".to_string(),
            )
            .await
            .unwrap();

        assert_eq!(saved.kind, MediaType::Episode);
        assert_eq!(saved.id, "episode:imdb/tt1/1/2");
        assert_eq!(saved.user_ref, "user-a");
        assert_eq!(saved.progress, 1500);
        assert_eq!(saved.parent.as_deref(), Some("serie:imdb/tt1"));
        assert_eq!(
            saved.modified,
            progress_rows(&store).await[&key("episode:imdb/tt1/1/2", "user-a")].1
        );
        assert!(saved.modified > 0);
    }

    #[tokio::test]
    async fn marking_watched_returns_the_removed_progress() {
        let store = progress_store().await;
        add_progress(&store, "imdb:tt1", "user-a", 1000).await;
        add_progress(&store, "imdb:tt1", "user-b", 2000).await;
        let watched = || WatchedForAdd {
            kind: MediaType::Movie,
            id: "imdb:tt1".to_string(),
            date: 1_700_000_000_000,
        };

        let cleared = store
            .add_watched(watched(), "user-a".to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cleared.id, "imdb:tt1");
        assert_eq!(cleared.user_ref, "user-a");
        assert_eq!(cleared.progress, 1000);

        // Only that user's position is removed; nothing is left to remove after.
        let rows = progress_rows(&store).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[&key("imdb:tt1", "user-b")].0, 2000);
        let again = store
            .add_watched(watched(), "user-a".to_string())
            .await
            .unwrap();
        assert_eq!(again, None);
    }

    #[tokio::test]
    async fn progress_rewrite_preserves_the_user_timestamp() {
        let store = progress_store().await;
        // Moved to a new id.
        add_progress(&store, "redseat:moved", "user-a", 1000).await;
        set_progress_modified(&store, "redseat:moved", "user-a", 5).await;
        // Merged into an existing row: the most recent position and its time win.
        add_progress(&store, "redseat:old", "user-a", 4000).await;
        set_progress_modified(&store, "redseat:old", "user-a", 20).await;
        add_progress(&store, "imdb:tt2", "user-a", 3000).await;
        set_progress_modified(&store, "imdb:tt2", "user-a", 10).await;

        let rewrite = |old_id: &str, new_id: &str| ProgressIdRewrite {
            kind: MediaType::Movie,
            old_id: old_id.to_string(),
            new_id: new_id.to_string(),
            new_parent: None,
            user_ref: "user-a".to_string(),
        };
        store
            .apply_history_rewrites(
                vec![],
                vec![
                    rewrite("redseat:moved", "imdb:tt1"),
                    rewrite("redseat:old", "imdb:tt2"),
                ],
            )
            .await
            .unwrap();

        let rows = progress_rows(&store).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[&key("imdb:tt1", "user-a")], (1000, 5));
        assert_eq!(rows[&key("imdb:tt2", "user-a")], (4000, 20));
    }

    #[tokio::test]
    async fn book_watched_rows_are_scoped_to_the_user() {
        let connection = Connection::open_in_memory().await.unwrap();
        migrate_database(&connection).await.unwrap();
        let store = SqliteStore {
            server_store: connection,
            libraries_stores: RwLock::new(HashMap::new()),
        };
        store
            .add_watched(
                WatchedForAdd {
                    kind: MediaType::Book,
                    id: "book:isbn13/9783161484100".to_string(),
                    date: 1_725_000_000_123,
                },
                "user-a".to_string(),
            )
            .await
            .unwrap();

        let query = HistoryQuery {
            types: vec![MediaType::Book],
            id: Some(RsIds::try_from("book:isbn13/9783161484100".to_string()).unwrap()),
            ..Default::default()
        };
        let user_a = store
            .get_watched(query.clone(), "user-a".to_string(), vec![])
            .await
            .unwrap();
        let user_b = store
            .get_watched(query, "user-b".to_string(), vec![])
            .await
            .unwrap();

        assert_eq!(user_a.len(), 1);
        assert!(user_b.is_empty());
    }

    #[tokio::test]
    async fn book_watched_rewrite_preserves_the_user_timestamp() {
        let connection = Connection::open_in_memory().await.unwrap();
        migrate_database(&connection).await.unwrap();
        let store = SqliteStore {
            server_store: connection,
            libraries_stores: RwLock::new(HashMap::new()),
        };
        store
            .add_watched(
                WatchedForAdd {
                    kind: MediaType::Book,
                    id: "book:redseat/book-local".to_string(),
                    date: 1_725_000_000_123,
                },
                "user-a".to_string(),
            )
            .await
            .unwrap();

        let (watched_count, progress_count) = store
            .apply_history_rewrites(
                vec![HistoryIdRewrite {
                    kind: MediaType::Book,
                    old_id: "book:redseat/book-local".to_string(),
                    new_id: "book:isbn13/9783161484100".to_string(),
                    user_ref: "user-a".to_string(),
                }],
                vec![],
            )
            .await
            .unwrap();

        let watched = store
            .get_watched(
                HistoryQuery {
                    types: vec![MediaType::Book],
                    id: Some(RsIds::try_from("book:isbn13/9783161484100".to_string()).unwrap()),
                    ..Default::default()
                },
                "user-a".to_string(),
                vec![],
            )
            .await
            .unwrap();

        assert_eq!((watched_count, progress_count), (1, 0));
        assert_eq!(watched.len(), 1);
        assert_eq!(watched[0].date, 1_725_000_000_123);
    }
}

///Upload key store
impl SqliteStore {
    pub async fn get_upload_key(&self, key: String) -> Result<UploadKey> {
        let keyc = key.clone();
        let row = self
            .server_store
            .call(move |conn| {
                let mut query = conn.prepare(
                    "SELECT id, library_ref, expiry, tags, people FROM uploadkeys where id = ?",
                )?;

                let rows = query.query_map(params![key], Self::row_to_uploadkey)?;
                let backups: Vec<UploadKey> =
                    rows.collect::<std::result::Result<Vec<UploadKey>, rusqlite::Error>>()?;
                Ok(backups)
            })
            .await?;
        let uploadkey = row.first().ok_or(SourcesError::UnableToFindUploadKey(
            "store".to_string(),
            keyc,
            "get_upload_key".to_string(),
        ))?;
        Ok(uploadkey.clone())
    }

    pub async fn get_upload_keys(&self) -> Result<Vec<UploadKey>> {
        let row = self
            .server_store
            .call(move |conn| {
                let mut query =
                    conn.prepare("SELECT id, library_ref, expiry, tags, people FROM uploadkeys")?;
                let rows = query.query_map([], Self::row_to_uploadkey)?;
                let keys: Vec<UploadKey> =
                    rows.collect::<std::result::Result<Vec<UploadKey>, rusqlite::Error>>()?;
                Ok(keys)
            })
            .await?;
        Ok(row)
    }

    pub async fn add_upload_key(&self, key: UploadKey) -> Result<()> {
        self.server_store.call(move |conn| {
            conn.execute(
                "INSERT INTO uploadkeys (id, library_ref, expiry, tags, people) VALUES (?, ?, ?, ?, ?)",
                params![key.id, key.library, key.expiry, key.tags, key.people],
            )?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_upload_key(&self, key_id: String) -> Result<()> {
        self.server_store
            .call(move |conn| {
                conn.execute("DELETE FROM uploadkeys WHERE id = ?", &[&key_id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    fn row_to_uploadkey(row: &Row) -> rusqlite::Result<UploadKey> {
        Ok(UploadKey {
            id: row.get(0)?,
            library: row.get(1)?,
            expiry: row.get(2)?,
            tags: row.get(3)?,
            people: row.get(4)?,
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
        let row = self
            .server_store
            .call(move |conn| {
                let mut query = conn.prepare(&format!(
                    "SELECT type, id, user_ref, date, modified  FROM Watched"
                ))?;

                let rows = query.query_map(params![], Self::row_to_watched)?;
                let backups: Vec<Watched> =
                    rows.collect::<std::result::Result<Vec<Watched>, rusqlite::Error>>()?;
                Ok(backups)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_watched(
        &self,
        query: HistoryQuery,
        user_id: String,
        other_users: Vec<String>,
    ) -> Result<Vec<Watched>> {
        let row = self
            .server_store
            .call(move |conn| {
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
                if other_users.len() > 0 {
                    let mut types = vec![];
                    types.push(SqlWhereType::Equal(
                        "user_ref".to_owned(),
                        Box::new(user_id),
                    ));
                    for other_user in other_users {
                        types.push(SqlWhereType::Equal(
                            "user_ref".to_owned(),
                            Box::new(other_user),
                        ));
                    }

                    where_query.add_where(SqlWhereType::Or(types));
                } else {
                    where_query.add_where(SqlWhereType::Equal(
                        "user_ref".to_owned(),
                        Box::new(user_id),
                    ));
                }

                if let Some(ids) = query.id {
                    let ids: Vec<String> = ids.into();
                    let ids = ids
                        .into_iter()
                        .map(|f| Box::new(f) as Box<dyn ToSql>)
                        .collect();
                    where_query.add_where(SqlWhereType::In("id".to_owned(), ids));
                }

                where_query.add_oder(OrderBuilder::new(query.sort.to_string(), query.order));

                let mut query = conn.prepare(&format!(
                    "SELECT type, id, user_ref, date, modified  FROM Watched {}{}",
                    where_query.format(),
                    where_query.format_order()
                ))?;

                let rows = query.query_map(where_query.values(), Self::row_to_watched)?;
                let backups: Vec<Watched> =
                    rows.collect::<std::result::Result<Vec<Watched>, rusqlite::Error>>()?;
                Ok(backups)
            })
            .await?;
        Ok(row)
    }

    /// Marks as watched and returns the resume position it removed, if there was one.
    pub async fn add_watched(
        &self,
        watched: WatchedForAdd,
        user_id: String,
    ) -> Result<Option<ViewProgress>> {
        let cleared = self
            .server_store
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO Watched (type, id, user_ref, date)
            VALUES (?, ? ,?, ?)",
                    params![watched.kind, watched.id, user_id, watched.date],
                )?;

                let cleared = conn
                    .query_row(
                        "SELECT type, id, user_ref, progress, parent, modified FROM progress
                     WHERE type = ? AND id = ? AND user_ref = ?",
                        params![watched.kind, watched.id, user_id],
                        Self::row_to_view_progress,
                    )
                    .optional()?;
                conn.execute(
                    "DELETE FROM progress where type = ? and id = ? and user_ref = ?",
                    params![watched.kind, watched.id, user_id,],
                )?;

                Ok(cleared)
            })
            .await?;
        Ok(cleared)
    }

    /// Deletes watched entries and returns the IDs that existed.
    pub async fn delete_watched(
        &self,
        kind: MediaType,
        ids: Vec<String>,
        user_ref: String,
    ) -> Result<Vec<String>> {
        let deleted_ids = self
            .server_store
            .call(move |conn| {
                let mut deleted_ids = Vec::new();
                for id in ids {
                    let rows_affected = conn.execute(
                        "DELETE FROM Watched WHERE type = ? AND id = ? AND user_ref = ?",
                        params![kind, &id, &user_ref],
                    )?;
                    if rows_affected > 0 {
                        deleted_ids.push(id);
                    }
                }
                Ok(deleted_ids)
            })
            .await?;
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

    pub async fn get_all_view_progress(
        &self,
        query: HistoryQuery,
        user_id: String,
    ) -> Result<Vec<ViewProgress>> {
        let row = self
            .server_store
            .call(move |conn| {
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
                where_query.add_where(SqlWhereType::Equal(
                    "user_ref".to_owned(),
                    Box::new(user_id),
                ));
                if let Some(ids) = query.id {
                    let ids: Vec<String> = ids.into();
                    let ids = ids
                        .into_iter()
                        .map(|f| Box::new(f) as Box<dyn ToSql>)
                        .collect();
                    where_query.add_where(SqlWhereType::In("id".to_owned(), ids));
                }

                where_query.add_oder(OrderBuilder::new(query.sort.to_string(), query.order));

                let mut query = conn.prepare(&format!(
                    "SELECT type, id, user_ref, progress, parent, modified  FROM progress {}{}",
                    where_query.format(),
                    where_query.format_order()
                ))?;

                let rows = query.query_map(where_query.values(), Self::row_to_view_progress)?;
                let backups: Vec<ViewProgress> =
                    rows.collect::<std::result::Result<Vec<ViewProgress>, rusqlite::Error>>()?;
                Ok(backups)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_all_view_progress_rows(&self) -> Result<Vec<ViewProgress>> {
        let row = self
            .server_store
            .call(move |conn| {
                let mut query = conn.prepare(
                    "SELECT type, id, user_ref, progress, parent, modified FROM progress",
                )?;
                let rows = query.query_map(params![], Self::row_to_view_progress)?;
                let progresses: Vec<ViewProgress> =
                    rows.collect::<std::result::Result<Vec<ViewProgress>, rusqlite::Error>>()?;
                Ok(progresses)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_view_progess(
        &self,
        ids: RsIds,
        user_id: String,
    ) -> Result<Option<ViewProgress>> {
        let row = self
            .server_store
            .call(move |conn| {
                let mut builder_query = RsQueryBuilder::new();

                builder_query.add_where(SqlWhereType::Equal(
                    "user_ref".to_owned(),
                    Box::new(user_id),
                ));

                let ids: Vec<String> = ids.into();
                let ids = ids
                    .into_iter()
                    .map(|f| Box::new(f) as Box<dyn ToSql>)
                    .collect();
                builder_query.add_where(SqlWhereType::In("id".to_owned(), ids));

                let mut query = conn.prepare(&format!(
                    "SELECT type, id, user_ref, progress, parent, modified  FROM progress {}",
                    builder_query.format()
                ))?;

                let row = query
                    .query_row(builder_query.values(), Self::row_to_view_progress)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    /// Saves the position and returns the stored row, with the time the triggers set.
    pub async fn add_view_progress(
        &self,
        progress: ViewProgressForAdd,
        user_ref: String,
    ) -> Result<ViewProgress> {
        let row = self
            .server_store
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO progress (type, id, user_ref, progress, parent)
            VALUES (?, ?, ? ,?,?)",
                    params![
                        progress.kind,
                        progress.id,
                        user_ref,
                        progress.progress,
                        progress.parent
                    ],
                )?;
                let row = conn.query_row(
                    "SELECT type, id, user_ref, progress, parent, modified FROM progress
                     WHERE type = ? AND id = ? AND user_ref = ?",
                    params![progress.kind, progress.id, user_ref],
                    Self::row_to_view_progress,
                )?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn apply_history_rewrites(
        &self,
        watched_rewrites: Vec<HistoryIdRewrite>,
        progress_rewrites: Vec<ProgressIdRewrite>,
    ) -> Result<(usize, usize)> {
        let row = self
            .server_store
            .call(move |conn| {
                let tx = conn.transaction()?;
                let mut watched_count = 0usize;
                let mut progress_count = 0usize;

                for rewrite in watched_rewrites {
                    if rewrite.old_id == rewrite.new_id {
                        continue;
                    }

                    let source = tx
                        .query_row(
                            "SELECT date, modified FROM Watched WHERE type = ? AND id = ? AND user_ref = ?",
                            params![rewrite.kind, rewrite.old_id, rewrite.user_ref],
                            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u64>(1)?)),
                        )
                        .optional()?;

                    let Some((source_date, source_modified)) = source else {
                        continue;
                    };

                    let existing = tx
                        .query_row(
                            "SELECT date, modified FROM Watched WHERE type = ? AND id = ? AND user_ref = ?",
                            params![rewrite.kind, rewrite.new_id, rewrite.user_ref],
                            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u64>(1)?)),
                        )
                        .optional()?;

                    if let Some((existing_date, existing_modified)) = existing {
                        let merged_date = if source_modified >= existing_modified {
                            source_date
                        } else {
                            existing_date
                        };
                        tx.execute(
                            "UPDATE Watched SET date = ? WHERE type = ? AND id = ? AND user_ref = ?",
                            params![
                                merged_date,
                                rewrite.kind,
                                rewrite.new_id,
                                rewrite.user_ref
                            ],
                        )?;
                    } else {
                        tx.execute(
                            "INSERT INTO Watched (type, id, user_ref, date) VALUES (?, ?, ?, ?)",
                            params![rewrite.kind, rewrite.new_id, rewrite.user_ref, source_date],
                        )?;
                    }
                    tx.execute(
                        "DELETE FROM Watched WHERE type = ? AND id = ? AND user_ref = ?",
                        params![rewrite.kind, rewrite.old_id, rewrite.user_ref],
                    )?;
                    watched_count += 1;
                }

                for rewrite in progress_rewrites {
                    if rewrite.old_id == rewrite.new_id {
                        continue;
                    }

                    let source = tx
                        .query_row(
                            "SELECT progress, parent, modified FROM progress WHERE type = ? AND id = ? AND user_ref = ?",
                            params![rewrite.kind, rewrite.old_id, rewrite.user_ref],
                            |row| {
                                Ok((
                                    row.get::<_, u64>(0)?,
                                    row.get::<_, Option<String>>(1)?,
                                    row.get::<_, u64>(2)?,
                                ))
                            },
                        )
                        .optional()?;

                    let Some((source_progress, source_parent, source_modified)) = source else {
                        continue;
                    };

                    let existing = tx
                        .query_row(
                            "SELECT progress, parent, modified FROM progress WHERE type = ? AND id = ? AND user_ref = ?",
                            params![rewrite.kind, rewrite.new_id, rewrite.user_ref],
                            |row| {
                                Ok((
                                    row.get::<_, u64>(0)?,
                                    row.get::<_, Option<String>>(1)?,
                                    row.get::<_, u64>(2)?,
                                ))
                            },
                        )
                        .optional()?;

                    if let Some((existing_progress, existing_parent, existing_modified)) = existing {
                        // The kept position keeps its own time.
                        let (merged_progress, merged_parent, merged_modified) =
                            if source_modified >= existing_modified {
                                (
                                    source_progress,
                                    rewrite
                                        .new_parent
                                        .clone()
                                        .or(source_parent.clone())
                                        .or(existing_parent.clone()),
                                    source_modified,
                                )
                            } else {
                                (
                                    existing_progress,
                                    rewrite.new_parent.clone().or(existing_parent.clone()),
                                    existing_modified,
                                )
                            };
                        tx.execute(
                            "UPDATE progress SET progress = ?, parent = ?, modified = ? WHERE type = ? AND id = ? AND user_ref = ?",
                            params![
                                merged_progress,
                                merged_parent,
                                merged_modified,
                                rewrite.kind,
                                rewrite.new_id,
                                rewrite.user_ref
                            ],
                        )?;
                        tx.execute(
                            "DELETE FROM progress WHERE type = ? AND id = ? AND user_ref = ?",
                            params![rewrite.kind, rewrite.old_id, rewrite.user_ref],
                        )?;
                    } else {
                        tx.execute(
                            "UPDATE progress SET id = ?, parent = ? WHERE type = ? AND id = ? AND user_ref = ?",
                            params![
                                rewrite.new_id,
                                rewrite.new_parent,
                                rewrite.kind,
                                rewrite.old_id,
                                rewrite.user_ref
                            ],
                        )?;
                    }
                    progress_count += 1;
                }

                tx.commit()?;
                Ok((watched_count, progress_count))
            })
            .await?;
        Ok(row)
    }

    pub async fn purge_watched_tombstones(&self) -> Result<usize> {
        let removed = self
            .server_store
            .call(|conn| Ok(conn.execute("DELETE FROM Watched WHERE date <= 0", [])?))
            .await?;
        Ok(removed)
    }

    pub async fn is_data_migration_complete(&self, name: &str) -> Result<bool> {
        let name = name.to_string();
        let complete = self
            .server_store
            .call(move |conn| {
                let complete = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM migrations WHERE name = ?)",
                    params![name],
                    |row| row.get(0),
                )?;
                Ok(complete)
            })
            .await?;
        Ok(complete)
    }

    pub async fn complete_data_migration(&self, name: &str) -> Result<()> {
        let name = name.to_string();
        self.server_store
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO migrations (name, up, down) SELECT ?, '', '' WHERE NOT EXISTS (SELECT 1 FROM migrations WHERE name = ?)",
                    params![name, name],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}
