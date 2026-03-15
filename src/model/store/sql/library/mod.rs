use crate::{
    model::{tags::TagQuery, ModelController},
    tools::log::{log_info, LogServiceType},
};
use rusqlite::{
    params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    ToSql,
};
use tokio_rusqlite::Connection;

use super::Result;

pub mod books;
pub mod channels;
pub mod deleted;
pub mod episodes;
pub mod media_progress;
pub mod media_ratings;
pub mod medias;
pub mod movie;
pub mod people;
pub mod request_processing;
pub mod series;
pub mod tags;

pub struct SqliteLibraryStore {
    connection: Connection,
}

// Constructor
impl SqliteLibraryStore {
    pub async fn new(connection: Connection) -> Result<Self> {
        let new = Self { connection };
        new.migrate().await?;
        Ok(new)
    }

    pub async fn migrate(&self) -> Result<usize> {
        let (initial_version, version) = self
            .connection
            .call(|conn| {
                let mut version =
                    conn.query_row("SELECT user_version FROM pragma_user_version;", [], |row| {
                        let version: usize = row.get(0)?;
                        Ok(version)
                    })?;
                let initial_version = version.clone();
                if version < 28 {
                    let initial = String::from_utf8_lossy(include_bytes!("001 - INITIAL.sql"));
                    conn.execute_batch(&initial)?;
                    version = 28;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }
                if version < 29 {
                    let initial = String::from_utf8_lossy(include_bytes!("029 - TAGPATH.sql"));
                    conn.execute_batch(&initial)?;
                    version = 29;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 30 {
                    let initial =
                        String::from_utf8_lossy(include_bytes!("030 - INSERT TRIGGERS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 30;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 31 {
                    let initial = String::from_utf8_lossy(include_bytes!("031 - MEDIAS INDEX.sql"));
                    conn.execute_batch(&initial)?;
                    version = 31;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 32 {
                    let initial = String::from_utf8_lossy(include_bytes!("032 - PROGRESS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 32;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 33 {
                    let initial = String::from_utf8_lossy(include_bytes!("033 - ORIGINAL.sql"));
                    conn.execute_batch(&initial)?;
                    version = 33;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 34 {
                    let initial = String::from_utf8_lossy(include_bytes!("034 - PEOPLE IDS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 34;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 35 {
                    let initial =
                        String::from_utf8_lossy(include_bytes!("035 - FACE RECOGNITION.sql"));
                    conn.execute_batch(&initial)?;
                    version = 35;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 36 {
                    let initial =
                        String::from_utf8_lossy(include_bytes!("036 - PEOPLE FACE REF.sql"));
                    conn.execute_batch(&initial)?;
                    version = 36;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 37 {
                    let initial = String::from_utf8_lossy(include_bytes!(
                        "037 - ADD SIMILARITY TO PEOPLE FACES.sql"
                    ));
                    conn.execute_batch(&initial)?;
                    version = 37;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 38 {
                    let initial = String::from_utf8_lossy(include_bytes!(
                        "038 - ADD FACE RECOGNITION ERROR.sql"
                    ));
                    conn.execute_batch(&initial)?;
                    version = 38;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 39 {
                    let initial = String::from_utf8_lossy(include_bytes!(
                        "039 - REQUEST PROCESSING.sql"
                    ));
                    conn.execute_batch(&initial)?;
                    version = 39;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 40 {
                    let initial = String::from_utf8_lossy(include_bytes!("040 - BOOK IDS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 40;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }
                if version < 41 {
                    let initial = String::from_utf8_lossy(include_bytes!("041 - BOOKS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 41;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 42 {
                    let initial = String::from_utf8_lossy(include_bytes!("042 - BOOK OTHERIDS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 42;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 43 {
                    let initial = String::from_utf8_lossy(include_bytes!("043 - PEOPLE OTHERIDS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 43;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 44 {
                    let initial = String::from_utf8_lossy(include_bytes!("044 - TAG OTHERIDS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 44;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 45 {
                    let initial = String::from_utf8_lossy(include_bytes!("045 - BOOK RELATIONS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 45;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 46 {
                    let initial = String::from_utf8_lossy(include_bytes!("046 - RATINGS TYPE.sql"));
                    conn.execute_batch(&initial)?;
                    version = 46;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                if version < 47 {
                    let initial = String::from_utf8_lossy(include_bytes!("047 - IPTV CHANNELS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 47;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }
                if version < 48 {
                    let initial = String::from_utf8_lossy(include_bytes!("048 - CHANNEL POSTERV.sql"));
                    conn.execute_batch(&initial)?;
                    version = 48;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }
                if version < 49 {
                    let initial = String::from_utf8_lossy(include_bytes!("049 - CHANNEL TAGS.sql"));
                    conn.execute_batch(&initial)?;
                    version = 49;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }
                if version < 50 {
                    let initial = String::from_utf8_lossy(include_bytes!("050 - CHANNEL VARIANT NAME.sql"));
                    conn.execute_batch(&initial)?;
                    version = 50;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }
                if version < 51 {
                    let initial = String::from_utf8_lossy(include_bytes!("051 - CASCADE DELETE.sql"));
                    conn.execute_batch(&initial)?;
                    version = 51;
                    conn.pragma_update(None, "user_version", version)?;
                    log_info(
                        LogServiceType::Database,
                        format!("Update Library Database to version: {}", version),
                    );
                }

                conn.execute("VACUUM;", params![])?;
                Ok((initial_version, version))
            })
            .await?;

        /*self.connection.call( |conn| {

            conn.execute("ALTER TABLE people ADD COLUMN generated INTEGER NOT NULL DEFAULT 0;", params![])?;

            Ok(())
        }).await?;*/

        if initial_version == 30 {
            log_info(
                LogServiceType::Database,
                format!("Update Library Database adding tag paths"),
            );
            let tags = self.get_tags(TagQuery::new_empty()).await?;
            let tags = ModelController::fill_tags_paths(None, "/", &tags);
            self.connection
                .call(|conn| {
                    for tag in tags {
                        conn.execute(
                            "UPDATE tags SET path = ? WHERE id = ?",
                            params![tag.path, tag.id],
                        )?;
                    }
                    Ok(())
                })
                .await?;
            log_info(
                LogServiceType::Database,
                format!("Update Library Database adding tag paths / Done"),
            );
        }

        Ok(version)
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteLibraryStore;

    #[tokio::test]
    async fn cascade_delete_triggers() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        let version = store.migrate().await.unwrap();
        assert_eq!(version, 51);

        // Set up: insert a book and a media attached to it
        store
            .connection
            .call(|conn| {
                conn.execute(
                    "INSERT INTO books (id, name) VALUES ('book-1', 'Test Book')",
                    [],
                )?;
                conn.execute(
                    "INSERT INTO medias (id, name, type, mimetype, book) VALUES ('m1', 'media-1', 'archive', 'application/octet-stream', 'book-1')",
                    [],
                )?;
                // Insert a movie and a media attached to it
                conn.execute(
                    "INSERT INTO movies (id, name) VALUES ('movie-1', 'Test Movie')",
                    [],
                )?;
                conn.execute(
                    "INSERT INTO medias (id, name, type, mimetype, movie) VALUES ('m2', 'media-2', 'video', 'video/mp4', 'movie-1')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        // Verify medias exist
        assert!(store.get_media("m1", None).await.unwrap().is_some());
        assert!(store.get_media("m2", None).await.unwrap().is_some());

        // Cascade delete: deleting book-1 should delete m1
        store.remove_book("book-1".to_string()).await.unwrap();
        assert!(store.get_media("m1", None).await.unwrap().is_none(), "media should be cascade deleted when book is deleted");

        // Cascade delete: deleting movie-1 should delete m2
        store.remove_movie("movie-1".to_string()).await.unwrap();
        assert!(store.get_media("m2", None).await.unwrap().is_none(), "media should be cascade deleted when movie is deleted");
    }
}
