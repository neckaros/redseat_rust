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

pub mod deleted;
pub mod episodes;
pub mod media_progress;
pub mod media_ratings;
pub mod medias;
pub mod movie;
pub mod people;
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

                conn.execute("VACUUM;", params![])?;
                conn.execute("DELETE FROM media_people_mapping where people_ref not in (select id from people) or media_ref not in (select id from medias);", []);
                conn.execute("DELETE FROM media_tag_mapping where tag_ref not in (select id from tags) or media_ref not in (select id from medias);", []);
                conn.execute("DELETE FROM media_serie_mapping where serie_ref not in (select id from series) or media_ref not in (select id from medias);", []);
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
