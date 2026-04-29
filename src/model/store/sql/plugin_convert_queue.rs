use std::str::FromStr;

use rusqlite::{params, params_from_iter, OptionalExtension, Row};

use crate::{
    domain::plugin_convert_queue::{
        PluginConvertQueueForInsert, PluginConvertQueueForUpdate, PluginConvertQueueItem,
        PluginConvertQueueStatus,
    },
    model::store::SqliteStore,
};

use super::Result;

const PLUGIN_CONVERT_COLUMNS: &str = "id, plugin_id, library_id, media_id, filename, request_json, status, plugin_job_id, progress, converted_id, error, requested_by, modified, added";

impl SqliteStore {
    fn row_to_plugin_convert_queue_item(row: &Row) -> rusqlite::Result<PluginConvertQueueItem> {
        let request_json: String = row.get(5)?;
        let status: String = row.get(6)?;
        Ok(PluginConvertQueueItem {
            id: row.get(0)?,
            plugin_id: row.get(1)?,
            library_id: row.get(2)?,
            media_id: row.get(3)?,
            filename: row.get(4)?,
            request: serde_json::from_str(&request_json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
            status: PluginConvertQueueStatus::from_str(&status).unwrap_or_default(),
            plugin_job_id: row.get(7)?,
            progress: row.get(8)?,
            converted_id: row.get(9)?,
            error: row.get(10)?,
            requested_by: row.get(11)?,
            modified: row.get(12)?,
            added: row.get(13)?,
        })
    }

    pub async fn add_plugin_convert_queue_item(
        &self,
        insert: PluginConvertQueueForInsert,
    ) -> Result<()> {
        let request_json = serde_json::to_string(&insert.request)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.server_store
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO plugin_convert_queue (id, plugin_id, library_id, media_id, filename, request_json, status, requested_by)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        insert.id,
                        insert.plugin_id,
                        insert.library_id,
                        insert.media_id,
                        insert.filename,
                        request_json,
                        PluginConvertQueueStatus::Queued.to_string(),
                        insert.requested_by,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn get_plugin_convert_queue_item(
        &self,
        id: &str,
    ) -> Result<Option<PluginConvertQueueItem>> {
        let id = id.to_string();
        let row = self
            .server_store
            .call(move |conn| {
                let row = conn
                    .query_row(
                        &format!(
                            "SELECT {} FROM plugin_convert_queue WHERE id = ?",
                            PLUGIN_CONVERT_COLUMNS
                        ),
                        params![id],
                        Self::row_to_plugin_convert_queue_item,
                    )
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn list_plugin_convert_queue_items(
        &self,
        library_id: &str,
        plugin_id: Option<String>,
        statuses: Option<Vec<PluginConvertQueueStatus>>,
    ) -> Result<Vec<PluginConvertQueueItem>> {
        let library_id = library_id.to_string();
        let row = self
            .server_store
            .call(move |conn| {
                let mut sql = format!(
                    "SELECT {} FROM plugin_convert_queue WHERE library_id = ?",
                    PLUGIN_CONVERT_COLUMNS
                );
                let mut values: Vec<String> = vec![library_id];
                if let Some(plugin_id) = plugin_id {
                    sql.push_str(" AND plugin_id = ?");
                    values.push(plugin_id);
                }
                if let Some(statuses) = statuses {
                    if !statuses.is_empty() {
                        sql.push_str(&format!(
                            " AND status IN ({})",
                            statuses.iter().map(|_| "?").collect::<Vec<_>>().join(", ")
                        ));
                        values.extend(statuses.into_iter().map(|s| s.to_string()));
                    }
                }
                sql.push_str(" ORDER BY added ASC");

                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(
                    params_from_iter(values.iter()),
                    Self::row_to_plugin_convert_queue_item,
                )?;
                let rows = rows.collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        Ok(row)
    }

    pub async fn list_active_plugin_convert_jobs(&self) -> Result<Vec<PluginConvertQueueItem>> {
        let row = self
            .server_store
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM plugin_convert_queue
                     WHERE status IN (?, ?, ?)
                     ORDER BY added ASC",
                    PLUGIN_CONVERT_COLUMNS
                ))?;
                let rows = stmt.query_map(
                    params![
                        PluginConvertQueueStatus::Submitted.to_string(),
                        PluginConvertQueueStatus::Downloading.to_string(),
                        PluginConvertQueueStatus::Processing.to_string(),
                    ],
                    Self::row_to_plugin_convert_queue_item,
                )?;
                let rows = rows.collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        Ok(row)
    }

    pub async fn count_active_plugin_convert_jobs(&self, plugin_id: &str) -> Result<usize> {
        let plugin_id = plugin_id.to_string();
        let row = self
            .server_store
            .call(move |conn| {
                let count: usize = conn.query_row(
                    "SELECT COUNT(*) FROM plugin_convert_queue
                     WHERE plugin_id = ? AND status IN (?, ?, ?)",
                    params![
                        plugin_id,
                        PluginConvertQueueStatus::Submitted.to_string(),
                        PluginConvertQueueStatus::Downloading.to_string(),
                        PluginConvertQueueStatus::Processing.to_string(),
                    ],
                    |row| row.get(0),
                )?;
                Ok(count)
            })
            .await?;
        Ok(row)
    }

    pub async fn list_queued_plugin_convert_jobs(
        &self,
        plugin_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<PluginConvertQueueItem>> {
        let plugin_id = plugin_id.to_string();
        let row = self
            .server_store
            .call(move |conn| {
                let sql = if limit.is_some() {
                    format!(
                        "SELECT {} FROM plugin_convert_queue
                         WHERE plugin_id = ? AND status = ?
                         ORDER BY added ASC LIMIT ?",
                        PLUGIN_CONVERT_COLUMNS
                    )
                } else {
                    format!(
                        "SELECT {} FROM plugin_convert_queue
                         WHERE plugin_id = ? AND status = ?
                         ORDER BY added ASC",
                        PLUGIN_CONVERT_COLUMNS
                    )
                };

                let mut stmt = conn.prepare(&sql)?;
                let queued = PluginConvertQueueStatus::Queued.to_string();
                let rows = if let Some(limit) = limit {
                    stmt.query_map(
                        params![plugin_id, queued, limit],
                        Self::row_to_plugin_convert_queue_item,
                    )?
                } else {
                    stmt.query_map(
                        params![plugin_id, queued],
                        Self::row_to_plugin_convert_queue_item,
                    )?
                };
                let rows = rows.collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        Ok(row)
    }

    pub async fn list_plugins_with_queued_convert_jobs(&self) -> Result<Vec<String>> {
        let row = self
            .server_store
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT plugin_id FROM plugin_convert_queue WHERE status = ?",
                )?;
                let rows = stmt.query_map(
                    params![PluginConvertQueueStatus::Queued.to_string()],
                    |row| row.get(0),
                )?;
                let rows = rows.collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        Ok(row)
    }

    pub async fn update_plugin_convert_queue_item(
        &self,
        id: &str,
        update: PluginConvertQueueForUpdate,
    ) -> Result<()> {
        let id = id.to_string();
        self.server_store
            .call(move |conn| {
                let mut updates = Vec::new();
                let mut values: Vec<Box<dyn rusqlite::ToSql + Send>> = Vec::new();

                if let Some(status) = update.status {
                    updates.push("status = ?");
                    values.push(Box::new(status.to_string()));
                }
                if let Some(plugin_job_id) = update.plugin_job_id {
                    updates.push("plugin_job_id = ?");
                    values.push(Box::new(plugin_job_id));
                }
                if let Some(progress) = update.progress {
                    updates.push("progress = ?");
                    values.push(Box::new(progress));
                }
                if let Some(converted_id) = update.converted_id {
                    updates.push("converted_id = ?");
                    values.push(Box::new(converted_id));
                }
                if let Some(error) = update.error {
                    updates.push("error = ?");
                    values.push(Box::new(error));
                }

                if !updates.is_empty() {
                    values.push(Box::new(id));
                    let sql = format!(
                        "UPDATE plugin_convert_queue SET {} WHERE id = ?",
                        updates.join(", ")
                    );
                    conn.execute(&sql, params_from_iter(values.iter().map(|v| v.as_ref())))?;
                }

                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn remove_plugin_convert_queue_item(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.server_store
            .call(move |conn| {
                conn.execute("DELETE FROM plugin_convert_queue WHERE id = ?", params![id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::RwLock};

    use rs_plugin_common_interfaces::video::{RsVideoFormat, VideoConvertRequest};
    use tokio_rusqlite::Connection;

    use crate::model::store::{sql::migrate_database, SqliteStore};

    use super::*;

    async fn test_store() -> SqliteStore {
        let connection = Connection::open_in_memory().await.unwrap();
        migrate_database(&connection).await.unwrap();
        SqliteStore {
            server_store: connection,
            libraries_stores: RwLock::new(HashMap::new()),
        }
    }

    fn insert_item(id: &str, plugin_id: &str) -> PluginConvertQueueForInsert {
        PluginConvertQueueForInsert {
            id: id.to_string(),
            plugin_id: plugin_id.to_string(),
            library_id: "library-1".to_string(),
            media_id: "media-1".to_string(),
            filename: "converted.mp4".to_string(),
            request: VideoConvertRequest {
                id: id.to_string(),
                format: RsVideoFormat::Mp4,
                ..Default::default()
            },
            requested_by: Some("user-1".to_string()),
        }
    }

    #[tokio::test]
    async fn plugin_convert_queue_store_lifecycle() {
        let store = test_store().await;
        store
            .add_plugin_convert_queue_item(insert_item("convert-1", "plugin-1"))
            .await
            .unwrap();

        let queued = store
            .list_plugin_convert_queue_items(
                "library-1",
                Some("plugin-1".to_string()),
                Some(vec![PluginConvertQueueStatus::Queued]),
            )
            .await
            .unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].request.id, "convert-1");

        assert_eq!(
            store
                .count_active_plugin_convert_jobs("plugin-1")
                .await
                .unwrap(),
            0
        );

        store
            .update_plugin_convert_queue_item(
                "convert-1",
                PluginConvertQueueForUpdate {
                    status: Some(PluginConvertQueueStatus::Submitted),
                    plugin_job_id: Some("job-1".to_string()),
                    progress: Some(0.25),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(
            store
                .count_active_plugin_convert_jobs("plugin-1")
                .await
                .unwrap(),
            1
        );

        let active = store.list_active_plugin_convert_jobs().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].plugin_job_id.as_deref(), Some("job-1"));

        store
            .remove_plugin_convert_queue_item("convert-1")
            .await
            .unwrap();
        assert!(store
            .get_plugin_convert_queue_item("convert-1")
            .await
            .unwrap()
            .is_none());
    }
}
