use rusqlite::{params, OptionalExtension, Row};

use crate::model::{
    libraries::{LibraryEncryptionItem, LibraryEncryptionJob},
    store::SqliteStore,
};

use super::Result;

impl SqliteStore {
    fn row_to_library_encryption_job(row: &Row) -> rusqlite::Result<LibraryEncryptionJob> {
        Ok(LibraryEncryptionJob {
            id: row.get(0)?,
            library_id: row.get(1)?,
            source_password: row.get(2)?,
            target_password: row.get(3)?,
            phase: row.get(4)?,
            snapshot_complete: row.get::<_, i64>(5)? != 0,
            total_items: row.get(6)?,
            completed_items: row.get(7)?,
            last_error: row.get(8)?,
            retry_count: row.get(9)?,
        })
    }

    fn row_to_library_encryption_item(row: &Row) -> rusqlite::Result<LibraryEncryptionItem> {
        Ok(LibraryEncryptionItem {
            id: row.get(0)?,
            job_id: row.get(1)?,
            kind: row.get(2)?,
            media_id: row.get(3)?,
            source: row.get(4)?,
            staged_source: row.get(5)?,
            state: row.get(6)?,
        })
    }

    pub async fn get_library_encryption_job(
        &self,
        library_id: &str,
    ) -> Result<Option<LibraryEncryptionJob>> {
        let library_id = library_id.to_string();
        Ok(self
            .server_store
            .call(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT id, library_id, source_password, target_password, phase,
                            snapshot_complete, total_items, completed_items, last_error, retry_count
                     FROM library_encryption_jobs WHERE library_id = ?",
                        params![library_id],
                        Self::row_to_library_encryption_job,
                    )
                    .optional()?)
            })
            .await?)
    }

    pub async fn list_active_library_encryption_jobs(&self) -> Result<Vec<LibraryEncryptionJob>> {
        Ok(self
            .server_store
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, library_id, source_password, target_password, phase,
                            snapshot_complete, total_items, completed_items, last_error, retry_count
                     FROM library_encryption_jobs
                     WHERE phase = 'running' ORDER BY created",
                )?;
                let rows = stmt.query_map([], Self::row_to_library_encryption_job)?;
                Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
            })
            .await?)
    }

    pub async fn create_library_encryption_job(&self, job: LibraryEncryptionJob) -> Result<()> {
        self.server_store
            .call(move |conn| {
                let transaction = conn.transaction()?;
                transaction.execute(
                    "DELETE FROM library_encryption_items WHERE job_id IN
                         (SELECT id FROM library_encryption_jobs WHERE library_id = ?)",
                    params![job.library_id],
                )?;
                transaction.execute(
                    "DELETE FROM library_encryption_jobs WHERE library_id = ?",
                    params![job.library_id],
                )?;
                transaction.execute(
                    "INSERT INTO library_encryption_jobs
                         (id, library_id, source_password, target_password, phase,
                          snapshot_complete, total_items, completed_items, last_error, retry_count)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        job.id,
                        job.library_id,
                        job.source_password,
                        job.target_password,
                        job.phase,
                        job.snapshot_complete,
                        job.total_items,
                        job.completed_items,
                        job.last_error,
                        job.retry_count,
                    ],
                )?;
                transaction.commit()?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn set_library_encryption_snapshot(
        &self,
        job_id: &str,
        items: Vec<LibraryEncryptionItem>,
    ) -> Result<()> {
        let job_id = job_id.to_string();
        self.server_store
            .call(move |conn| {
                let transaction = conn.transaction()?;
                for item in &items {
                    transaction.execute(
                        "INSERT OR IGNORE INTO library_encryption_items
                             (id, job_id, kind, media_id, source, staged_source, state)
                         VALUES (?, ?, ?, ?, ?, ?, ?)",
                        params![
                            item.id,
                            item.job_id,
                            item.kind,
                            item.media_id,
                            item.source,
                            item.staged_source,
                            item.state,
                        ],
                    )?;
                }
                transaction.execute(
                    "UPDATE library_encryption_jobs
                     SET snapshot_complete = 1, total_items = ?, modified = unixepoch()
                     WHERE id = ?",
                    params![items.len() as u64, job_id],
                )?;
                transaction.commit()?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_library_encryption_items(
        &self,
        job_id: &str,
    ) -> Result<Vec<LibraryEncryptionItem>> {
        let job_id = job_id.to_string();
        Ok(self
            .server_store
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, job_id, kind, media_id, source, staged_source, state
                     FROM library_encryption_items WHERE job_id = ? ORDER BY rowid",
                )?;
                let rows = stmt.query_map(params![job_id], Self::row_to_library_encryption_item)?;
                Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
            })
            .await?)
    }

    pub async fn mark_library_encryption_item_prepared(
        &self,
        item_id: &str,
        staged_source: &str,
    ) -> Result<()> {
        let item_id = item_id.to_string();
        let staged_source = staged_source.to_string();
        self.server_store
            .call(move |conn| {
                conn.execute(
                    "UPDATE library_encryption_items SET staged_source = ?, state = 'prepared'
                     WHERE id = ? AND state = 'pending'",
                    params![staged_source, item_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn mark_library_encryption_item_committed(&self, item_id: &str) -> Result<()> {
        let item_id = item_id.to_string();
        self.server_store
            .call(move |conn| {
                let transaction = conn.transaction()?;
                let job_id: String = transaction.query_row(
                    "SELECT job_id FROM library_encryption_items WHERE id = ?",
                    params![item_id],
                    |row| row.get(0),
                )?;
                transaction.execute(
                    "UPDATE library_encryption_items SET state = 'committed' WHERE id = ?",
                    params![item_id],
                )?;
                transaction.execute(
                    "UPDATE library_encryption_jobs
                     SET completed_items = (SELECT COUNT(*) FROM library_encryption_items
                                            WHERE job_id = ? AND state = 'committed'),
                         modified = unixepoch()
                     WHERE id = ?",
                    params![job_id, job_id],
                )?;
                transaction.commit()?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn set_library_encryption_error(
        &self,
        job_id: &str,
        error: Option<String>,
    ) -> Result<()> {
        let job_id = job_id.to_string();
        self.server_store
            .call(move |conn| {
                conn.execute(
                    "UPDATE library_encryption_jobs SET last_error = ?, modified = unixepoch()
                     WHERE id = ?",
                    params![error, job_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn increment_library_encryption_retry(&self, job_id: &str) -> Result<u32> {
        let job_id = job_id.to_string();
        Ok(self
            .server_store
            .call(move |conn| {
                conn.execute(
                    "UPDATE library_encryption_jobs SET retry_count = retry_count + 1,
                         modified = unixepoch() WHERE id = ? AND phase = 'running'",
                    params![job_id],
                )?;
                Ok(conn.query_row(
                    "SELECT retry_count FROM library_encryption_jobs WHERE id = ?",
                    params![job_id],
                    |row| row.get(0),
                )?)
            })
            .await?)
    }

    pub async fn fail_library_encryption_job(
        &self,
        job_id: &str,
        error: String,
    ) -> Result<()> {
        let job_id = job_id.to_string();
        self.server_store
            .call(move |conn| {
                conn.execute(
                    "UPDATE library_encryption_jobs
                     SET phase = 'failed', last_error = ?, modified = unixepoch()
                     WHERE id = ? AND phase = 'running'",
                    params![error, job_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn complete_library_encryption_job(&self, job_id: &str) -> Result<()> {
        let job_id = job_id.to_string();
        self.server_store
            .call(move |conn| {
                conn.execute(
                    "UPDATE library_encryption_jobs
                     SET phase = 'completed', source_password = NULL, target_password = NULL,
                         last_error = NULL, modified = unixepoch() WHERE id = ?",
                    params![job_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn set_library_password(
        &self,
        library_id: &str,
        password: Option<String>,
    ) -> Result<()> {
        let library_id = library_id.to_string();
        self.server_store
            .call(move |conn| {
                conn.execute(
                    "UPDATE Libraries SET password = ? WHERE id = ?",
                    params![password, library_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::RwLock};

    use tokio_rusqlite::Connection;

    use crate::model::{
        libraries::{LibraryEncryptionItem, LibraryEncryptionJob},
        store::{sql::migrate_database, SqliteStore},
    };

    async fn test_store() -> SqliteStore {
        let connection = Connection::open_in_memory().await.unwrap();
        migrate_database(&connection).await.unwrap();
        connection
            .call(|conn| {
                conn.execute(
                    "INSERT INTO Libraries (id, name, type, source, root, settings)
                     VALUES ('library-1', 'Movies', 'movies', 'PathProvider', '', '{}')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        SqliteStore {
            server_store: connection,
            libraries_stores: RwLock::new(HashMap::new()),
        }
    }

    #[tokio::test]
    async fn encryption_job_checkpoints_survive_store_roundtrips() {
        let store = test_store().await;
        let job = LibraryEncryptionJob {
            id: "job-1".to_string(),
            library_id: "library-1".to_string(),
            source_password: Some("old-password".to_string()),
            target_password: Some("new-password".to_string()),
            phase: "running".to_string(),
            snapshot_complete: false,
            total_items: 0,
            completed_items: 0,
            last_error: None,
            retry_count: 0,
        };
        store
            .create_library_encryption_job(job.clone())
            .await
            .unwrap();
        assert_eq!(store.list_active_library_encryption_jobs().await.unwrap().len(), 1);

        let item = LibraryEncryptionItem {
            id: "item-1".to_string(),
            job_id: job.id.clone(),
            kind: "local".to_string(),
            media_id: Some("media-1".to_string()),
            source: "/media/original".to_string(),
            staged_source: None,
            state: "pending".to_string(),
        };
        store
            .set_library_encryption_snapshot(&job.id, vec![item])
            .await
            .unwrap();
        store
            .mark_library_encryption_item_prepared("item-1", "/media/staged")
            .await
            .unwrap();
        store
            .mark_library_encryption_item_committed("item-1")
            .await
            .unwrap();

        let checkpoint = store
            .get_library_encryption_job("library-1")
            .await
            .unwrap()
            .unwrap();
        assert!(checkpoint.snapshot_complete);
        assert_eq!(checkpoint.total_items, 1);
        assert_eq!(checkpoint.completed_items, 1);
        let items = store
            .list_library_encryption_items(&job.id)
            .await
            .unwrap();
        assert_eq!(items[0].state, "committed");
        assert_eq!(items[0].staged_source.as_deref(), Some("/media/staged"));

        assert_eq!(
            store
                .increment_library_encryption_retry(&job.id)
                .await
                .unwrap(),
            1
        );

        store
            .fail_library_encryption_job(&job.id, "scheduler unavailable".to_string())
            .await
            .unwrap();
        let failed = store
            .get_library_encryption_job("library-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed.phase, "failed");
        assert_eq!(failed.last_error.as_deref(), Some("scheduler unavailable"));
        assert!(store.list_active_library_encryption_jobs().await.unwrap().is_empty());

        store
            .set_library_password("library-1", job.target_password.clone())
            .await
            .unwrap();
        store
            .complete_library_encryption_job(&job.id)
            .await
            .unwrap();
        let completed = store
            .get_library_encryption_job("library-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed.phase, "completed");
        assert!(completed.source_password.is_none());
        assert!(completed.target_password.is_none());
        assert_eq!(
            store
                .get_library("library-1")
                .await
                .unwrap()
                .unwrap()
                .password
                .as_deref(),
            Some("new-password")
        );
    }
}
