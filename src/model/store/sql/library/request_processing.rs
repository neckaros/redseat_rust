use rusqlite::{params, OptionalExtension, Row};
use super::{Result, SqliteLibraryStore};
use crate::domain::request_processing::{RsRequestProcessing, RsRequestProcessingForInsert, RsRequestProcessingForUpdate};
use rs_plugin_common_interfaces::request::RsProcessingStatus;
use std::str::FromStr;

impl SqliteLibraryStore {
    fn row_to_request_processing(row: &Row) -> rusqlite::Result<RsRequestProcessing> {
        let status_str: String = row.get(4)?;
        let original_request_json: Option<String> = row.get(8)?;
        Ok(RsRequestProcessing {
            id: row.get(0)?,
            processing_id: row.get(1)?,
            plugin_id: row.get(2)?,
            progress: row.get(3)?,
            status: RsProcessingStatus::from_str(&status_str).unwrap_or_default(),
            error: row.get(5)?,
            eta: row.get(6)?,
            media_ref: row.get(7)?,
            original_request: original_request_json.and_then(|s| serde_json::from_str(&s).ok()),
            modified: row.get(9)?,
            added: row.get(10)?,
        })
    }

    pub async fn get_request_processing(&self, id: &str) -> Result<Option<RsRequestProcessing>> {
        let id = id.to_string();
        let result = self.connection.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, processing_id, plugin_id, progress, status, error, eta, media_ref, original_request, modified, added
                 FROM request_processing WHERE id = ?"
            )?;
            let result = stmt.query_row(params![id], Self::row_to_request_processing).optional()?;
            Ok(result)
        }).await?;
        Ok(result)
    }

    pub async fn get_request_processings_by_status(&self, status: RsProcessingStatus) -> Result<Vec<RsRequestProcessing>> {
        let status_str = status.to_string();
        let results = self.connection.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, processing_id, plugin_id, progress, status, error, eta, media_ref, original_request, modified, added
                 FROM request_processing WHERE status = ? ORDER BY added DESC"
            )?;
            let rows = stmt.query_map(params![status_str], Self::row_to_request_processing)?;
            let results: Vec<RsRequestProcessing> = rows.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(results)
        }).await?;
        Ok(results)
    }

    pub async fn get_all_active_request_processings(&self) -> Result<Vec<RsRequestProcessing>> {
        let results = self.connection.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, processing_id, plugin_id, progress, status, error, eta, media_ref, original_request, modified, added
                 FROM request_processing WHERE status IN ('pending', 'processing') ORDER BY added DESC"
            )?;
            let rows = stmt.query_map([], Self::row_to_request_processing)?;
            let results: Vec<RsRequestProcessing> = rows.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(results)
        }).await?;
        Ok(results)
    }

    pub async fn add_request_processing(&self, insert: RsRequestProcessingForInsert) -> Result<()> {
        let original_request_json = insert.original_request
            .as_ref()
            .and_then(|r| serde_json::to_string(r).ok());
        self.connection.call(move |conn| {
            conn.execute(
                "INSERT INTO request_processing (id, processing_id, plugin_id, eta, media_ref, original_request)
                 VALUES (?, ?, ?, ?, ?, ?)",
                params![
                    insert.id,
                    insert.processing_id,
                    insert.plugin_id,
                    insert.eta,
                    insert.media_ref,
                    original_request_json,
                ],
            )?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn update_request_processing(&self, id: &str, update: RsRequestProcessingForUpdate) -> Result<()> {
        let id = id.to_string();
        self.connection.call(move |conn| {
            let mut updates = vec![];
            let mut values: Vec<Box<dyn rusqlite::ToSql + Send>> = vec![];

            if let Some(progress) = update.progress {
                updates.push("progress = ?");
                values.push(Box::new(progress as i64));
            }
            if let Some(status) = update.status {
                updates.push("status = ?");
                values.push(Box::new(status.to_string()));
            }
            if let Some(ref error) = update.error {
                updates.push("error = ?");
                values.push(Box::new(error.clone()));
            }
            if let Some(eta) = update.eta {
                updates.push("eta = ?");
                values.push(Box::new(eta));
            }

            if !updates.is_empty() {
                values.push(Box::new(id));
                let sql = format!("UPDATE request_processing SET {} WHERE id = ?", updates.join(", "));
                conn.execute(&sql, rusqlite::params_from_iter(values.iter().map(|v| v.as_ref())))?;
            }
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn remove_request_processing(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.connection.call(move |conn| {
            conn.execute("DELETE FROM request_processing WHERE id = ?", params![id])?;
            Ok(())
        }).await?;
        Ok(())
    }
}
