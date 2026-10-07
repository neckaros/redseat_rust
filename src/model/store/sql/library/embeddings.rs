use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{Result, SqliteLibraryStore};
use crate::tools::get_time;

/// Medias an embedder can process: photos and videos.
const EMBEDDABLE_TYPES: &str = "('photo', 'video')";

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaEmbeddingCounts {
    /// Photos and videos in the library.
    pub total: u64,
    pub embedded: u64,
    /// Still to embed, including failed medias that will be tried again.
    pub pending: u64,
    /// Gave up after too many failures.
    pub failed: u64,
}

impl SqliteLibraryStore {
    /// Stores the vector of a media for an embedder and forgets its earlier failures.
    ///
    /// Returns false, writing nothing, when the media no longer exists: it can be
    /// deleted while its vector is computed, after the delete trigger has run.
    pub async fn add_media_embedding(
        &self,
        media_id: String,
        embedder: String,
        embedding: Vec<u8>,
    ) -> Result<bool> {
        let written = self
            .connection
            .call(move |conn| {
                let tx = conn.transaction()?;
                let written = tx.execute(
                    "INSERT OR REPLACE INTO media_embeddings (media_ref, embedder, embedding)
                     SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM medias WHERE id = ?1)",
                    params![media_id, embedder, embedding],
                )?;
                tx.execute(
                    "DELETE FROM media_embedding_errors WHERE media_ref = ? AND embedder = ?",
                    params![media_id, embedder],
                )?;
                tx.commit()?;
                Ok(written > 0)
            })
            .await?;
        Ok(written)
    }

    pub async fn get_media_embedding(
        &self,
        media_id: String,
        embedder: String,
    ) -> Result<Option<Vec<u8>>> {
        let res = self
            .connection
            .call(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT embedding FROM media_embeddings WHERE media_ref = ? AND embedder = ?",
                        params![media_id, embedder],
                        |row| row.get::<_, Vec<u8>>(0),
                    )
                    .optional()?)
            })
            .await?;
        Ok(res)
    }

    pub async fn remove_media_embedding(&self, media_id: String, embedder: String) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM media_embeddings WHERE media_ref = ? AND embedder = ?",
                    params![media_id, embedder],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// All vectors of an embedder as `(media ids, concatenated prefixes)`: the first
    /// `prefix` bytes of each vector, in id order. Shorter rows are skipped.
    pub async fn get_media_embedding_prefixes(
        &self,
        embedder: String,
        prefix: usize,
    ) -> Result<(Vec<String>, Vec<u8>)> {
        let res = self
            .connection
            .call(move |conn| {
                let count: usize = conn.query_row(
                    "SELECT COUNT(*) FROM media_embeddings WHERE embedder = ?",
                    params![embedder],
                    |row| row.get(0),
                )?;
                let mut ids = Vec::with_capacity(count);
                let mut data = Vec::with_capacity(count * prefix);
                let mut stmt = conn.prepare(
                    "SELECT media_ref, embedding FROM media_embeddings WHERE embedder = ?",
                )?;
                let mut rows = stmt.query(params![embedder])?;
                while let Some(row) = rows.next()? {
                    let embedding = row.get_ref(1)?.as_blob().map_err(rusqlite::Error::from)?;
                    if let Some(embedding) = embedding.get(..prefix) {
                        ids.push(row.get::<_, String>(0)?);
                        data.extend_from_slice(embedding);
                    }
                }
                Ok((ids, data))
            })
            .await?;
        Ok(res)
    }

    /// Photos and videos with no vector for `embedder`, newest first. This is the
    /// work queue: anything that embeds medias claims its work here.
    ///
    /// A media that failed is offered again until it reaches `max_attempts`, but
    /// not before `retry_failed_before` (ms), so one pass tries it at most once.
    pub async fn get_medias_pending_embedding(
        &self,
        embedder: String,
        max_attempts: u32,
        retry_failed_before: i64,
        limit: usize,
    ) -> Result<Vec<String>> {
        let limit = limit as i64;
        let res = self
            .connection
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT m.id FROM medias m
                     WHERE m.type IN {EMBEDDABLE_TYPES}
                       AND NOT EXISTS (SELECT 1 FROM media_embeddings e
                            WHERE e.media_ref = m.id AND e.embedder = ?1)
                       AND NOT EXISTS (SELECT 1 FROM media_embedding_errors f
                            WHERE f.media_ref = m.id AND f.embedder = ?1
                              AND (f.attempts >= ?2 OR f.modified >= ?3))
                     ORDER BY m.added DESC, m.id LIMIT ?4"
                ))?;
                let rows = stmt.query_map(
                    params![embedder, max_attempts, retry_failed_before, limit],
                    |row| row.get::<_, String>(0),
                )?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await?;
        Ok(res)
    }

    pub async fn count_media_embeddings(
        &self,
        embedder: String,
        max_attempts: u32,
    ) -> Result<MediaEmbeddingCounts> {
        let res = self
            .connection
            .call(move |conn| {
                let counts = conn.query_row(
                    &format!(
                        "SELECT COUNT(*),
                            COALESCE(SUM(EXISTS (SELECT 1 FROM media_embeddings e
                                WHERE e.media_ref = m.id AND e.embedder = ?1)), 0),
                            COALESCE(SUM(EXISTS (SELECT 1 FROM media_embedding_errors f
                                WHERE f.media_ref = m.id AND f.embedder = ?1 AND f.attempts >= ?2)
                              AND NOT EXISTS (SELECT 1 FROM media_embeddings e
                                WHERE e.media_ref = m.id AND e.embedder = ?1)), 0)
                         FROM medias m WHERE m.type IN {EMBEDDABLE_TYPES}"
                    ),
                    params![embedder, max_attempts],
                    |row| {
                        let total: u64 = row.get(0)?;
                        let embedded: u64 = row.get(1)?;
                        let failed: u64 = row.get(2)?;
                        Ok(MediaEmbeddingCounts {
                            total,
                            embedded,
                            pending: total.saturating_sub(embedded + failed),
                            failed,
                        })
                    },
                )?;
                Ok(counts)
            })
            .await?;
        Ok(res)
    }

    /// Records a failed attempt. Returns the number of attempts so far, 0 when the
    /// media no longer exists (nothing is recorded then).
    pub async fn mark_media_embedding_error(
        &self,
        media_id: String,
        embedder: String,
        error: String,
    ) -> Result<u32> {
        let now = get_time().as_millis() as i64;
        let res = self
            .connection
            .call(move |conn| {
                let attempts = conn
                    .query_row(
                        "INSERT INTO media_embedding_errors (media_ref, embedder, error, attempts, modified)
                         SELECT ?1, ?2, ?3, 1, ?4 WHERE EXISTS (SELECT 1 FROM medias WHERE id = ?1)
                         ON CONFLICT (media_ref, embedder) DO UPDATE SET
                            error = excluded.error, attempts = attempts + 1, modified = excluded.modified
                         RETURNING attempts",
                        params![media_id, embedder, error, now],
                        |row| row.get::<_, u32>(0),
                    )
                    .optional()?;
                Ok(attempts.unwrap_or(0))
            })
            .await?;
        Ok(res)
    }

    /// Lets failed medias be tried again from scratch.
    pub async fn clear_media_embedding_errors(&self, embedder: String) -> Result<usize> {
        let res = self
            .connection
            .call(move |conn| {
                Ok(conn.execute(
                    "DELETE FROM media_embedding_errors WHERE embedder = ?",
                    params![embedder],
                )?)
            })
            .await?;
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::media::{FileType, MediaForAdd};

    const EMBEDDER: &str = "test-embedder/v1";
    const MAX_ATTEMPTS: u32 = 2;

    async fn store_with_medias() -> SqliteLibraryStore {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        for (id, kind) in [
            ("photo-1", FileType::Photo),
            ("photo-2", FileType::Photo),
            ("video-1", FileType::Video),
            ("archive-1", FileType::Archive),
        ] {
            store
                .add_media(
                    MediaForAdd {
                        name: id.to_string(),
                        kind,
                        ..Default::default()
                    }
                    .into_insert_with_id(id.to_string()),
                )
                .await
                .unwrap();
        }
        store
    }

    async fn pending(store: &SqliteLibraryStore, retry_failed_before: i64) -> Vec<String> {
        let mut ids = store
            .get_medias_pending_embedding(
                EMBEDDER.to_string(),
                MAX_ATTEMPTS,
                retry_failed_before,
                10,
            )
            .await
            .unwrap();
        ids.sort();
        ids
    }

    async fn counts(store: &SqliteLibraryStore) -> MediaEmbeddingCounts {
        store
            .count_media_embeddings(EMBEDDER.to_string(), MAX_ATTEMPTS)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn migration_creates_embedding_tables() {
        let store = store_with_medias().await;
        let tables: Vec<String> = store
            .connection
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT name FROM sqlite_master WHERE name LIKE 'media_embedding%' ORDER BY name",
                )?;
                let rows = stmt.query_map([], |row| row.get(0))?;
                Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
            })
            .await
            .unwrap();
        assert_eq!(tables, vec!["media_embedding_errors", "media_embeddings"]);
    }

    #[tokio::test]
    async fn pending_lists_photos_and_videos_without_a_vector_for_the_embedder() {
        let store = store_with_medias().await;
        assert_eq!(
            pending(&store, i64::MAX).await,
            vec!["photo-1", "photo-2", "video-1"]
        );
        assert_eq!(
            counts(&store).await,
            MediaEmbeddingCounts {
                total: 3,
                embedded: 0,
                pending: 3,
                failed: 0
            }
        );

        store
            .add_media_embedding("photo-1".into(), EMBEDDER.into(), vec![1, 2, 3])
            .await
            .unwrap();
        // A vector from another embedder does not make the media done for this one.
        store
            .add_media_embedding("photo-2".into(), "other/v1".into(), vec![9, 9, 9])
            .await
            .unwrap();

        assert_eq!(pending(&store, i64::MAX).await, vec!["photo-2", "video-1"]);
        let limited = store
            .get_medias_pending_embedding(EMBEDDER.into(), MAX_ATTEMPTS, i64::MAX, 1)
            .await
            .unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(counts(&store).await.embedded, 1);
        assert_eq!(counts(&store).await.pending, 2);
    }

    #[tokio::test]
    async fn embeddings_roundtrip_and_load_as_prefixes() {
        let store = store_with_medias().await;
        store
            .add_media_embedding("photo-1".into(), EMBEDDER.into(), vec![1, 2, 3, 4])
            .await
            .unwrap();
        store
            .add_media_embedding("video-1".into(), EMBEDDER.into(), vec![255, 6, 7, 8])
            .await
            .unwrap();
        // Replaced, not duplicated.
        store
            .add_media_embedding("photo-1".into(), EMBEDDER.into(), vec![5, 5, 5, 5])
            .await
            .unwrap();
        store
            .add_media_embedding("photo-2".into(), EMBEDDER.into(), vec![1])
            .await
            .unwrap();
        store
            .add_media_embedding("photo-2".into(), "other/v1".into(), vec![9, 9, 9, 9])
            .await
            .unwrap();

        assert_eq!(
            store
                .get_media_embedding("photo-1".into(), EMBEDDER.into())
                .await
                .unwrap(),
            Some(vec![5, 5, 5, 5])
        );
        assert_eq!(
            store
                .get_media_embedding("archive-1".into(), EMBEDDER.into())
                .await
                .unwrap(),
            None
        );

        // The one-byte row is too short for a prefix of 2 and is left out.
        let (ids, data) = store
            .get_media_embedding_prefixes(EMBEDDER.into(), 2)
            .await
            .unwrap();
        assert_eq!(ids, vec!["photo-1", "video-1"]);
        assert_eq!(data, vec![5, 5, 255, 6]);

        store
            .remove_media_embedding("photo-1".into(), EMBEDDER.into())
            .await
            .unwrap();
        let (ids, _) = store
            .get_media_embedding_prefixes(EMBEDDER.into(), 2)
            .await
            .unwrap();
        assert_eq!(ids, vec!["video-1"]);
    }

    #[tokio::test]
    async fn failures_are_retried_once_per_pass_then_given_up() {
        let store = store_with_medias().await;
        let pass_1 = get_time().as_millis() as i64;
        let attempts = store
            .mark_media_embedding_error("photo-1".into(), EMBEDDER.into(), "boom".into())
            .await
            .unwrap();
        assert_eq!(attempts, 1);

        // Not offered again during the pass that just failed it...
        assert_eq!(pending(&store, pass_1).await, vec!["photo-2", "video-1"]);
        // ...but still counted as pending, and offered to a later pass.
        assert_eq!(counts(&store).await.pending, 3);
        assert_eq!(counts(&store).await.failed, 0);
        assert_eq!(
            pending(&store, i64::MAX).await,
            vec!["photo-1", "photo-2", "video-1"]
        );

        let attempts = store
            .mark_media_embedding_error("photo-1".into(), EMBEDDER.into(), "boom again".into())
            .await
            .unwrap();
        assert_eq!(attempts, MAX_ATTEMPTS);
        assert_eq!(pending(&store, i64::MAX).await, vec!["photo-2", "video-1"]);
        assert_eq!(
            counts(&store).await,
            MediaEmbeddingCounts {
                total: 3,
                embedded: 0,
                pending: 2,
                failed: 1
            }
        );

        // Failures are per embedder.
        let other = store
            .get_medias_pending_embedding("other/v1".into(), MAX_ATTEMPTS, i64::MAX, 10)
            .await
            .unwrap();
        assert_eq!(other.len(), 3);

        // A success clears the failure.
        store
            .add_media_embedding("photo-1".into(), EMBEDDER.into(), vec![1])
            .await
            .unwrap();
        assert_eq!(
            counts(&store).await,
            MediaEmbeddingCounts {
                total: 3,
                embedded: 1,
                pending: 2,
                failed: 0
            }
        );

        store
            .mark_media_embedding_error("photo-2".into(), EMBEDDER.into(), "boom".into())
            .await
            .unwrap();
        store
            .mark_media_embedding_error("photo-2".into(), EMBEDDER.into(), "boom".into())
            .await
            .unwrap();
        assert_eq!(counts(&store).await.failed, 1);
        assert_eq!(
            store
                .clear_media_embedding_errors(EMBEDDER.into())
                .await
                .unwrap(),
            1
        );
        assert_eq!(pending(&store, i64::MAX).await, vec!["photo-2", "video-1"]);
    }

    #[tokio::test]
    async fn deleting_a_media_removes_its_embedding_rows() {
        let store = store_with_medias().await;
        store
            .add_media_embedding("photo-1".into(), EMBEDDER.into(), vec![1, 2])
            .await
            .unwrap();
        store
            .add_media_embedding("photo-2".into(), EMBEDDER.into(), vec![3, 4])
            .await
            .unwrap();
        store
            .mark_media_embedding_error("video-1".into(), EMBEDDER.into(), "boom".into())
            .await
            .unwrap();

        store.remove_media("photo-1".to_string()).await.unwrap();
        store.remove_media("video-1".to_string()).await.unwrap();

        let (ids, _) = store
            .get_media_embedding_prefixes(EMBEDDER.into(), 2)
            .await
            .unwrap();
        assert_eq!(ids, vec!["photo-2"]);
        let errors: u64 = store
            .connection
            .call(|conn| {
                Ok(
                    conn.query_row("SELECT COUNT(*) FROM media_embedding_errors", [], |row| {
                        row.get(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(errors, 0);
        assert_eq!(counts(&store).await.total, 1);
    }

    #[tokio::test]
    async fn nothing_is_written_for_a_media_deleted_meanwhile() {
        let store = store_with_medias().await;
        store.remove_media("photo-1".to_string()).await.unwrap();

        assert!(!store
            .add_media_embedding("photo-1".into(), EMBEDDER.into(), vec![1, 2])
            .await
            .unwrap());
        let attempts = store
            .mark_media_embedding_error("photo-1".into(), EMBEDDER.into(), "boom".into())
            .await
            .unwrap();
        assert_eq!(attempts, 0);
        let rows: u64 = store
            .connection
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT (SELECT COUNT(*) FROM media_embeddings)
                          + (SELECT COUNT(*) FROM media_embedding_errors)",
                    [],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(rows, 0);

        // A media that is still there is written as before.
        assert!(store
            .add_media_embedding("photo-2".into(), EMBEDDER.into(), vec![1, 2])
            .await
            .unwrap());
        let attempts = store
            .mark_media_embedding_error("video-1".into(), EMBEDDER.into(), "boom".into())
            .await
            .unwrap();
        assert_eq!(attempts, 1);
    }
}
