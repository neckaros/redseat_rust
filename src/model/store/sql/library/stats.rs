use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Result, SqliteLibraryStore};

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMediaTypeStats {
    pub count: u64,
    /// Bytes.
    pub size: u64,
    /// Seconds. Left out when no media of the type has a duration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMediaStats {
    pub count: u64,
    pub size: u64,
    /// Keyed by media type (`photo`, `video`...): only the types the library holds.
    pub by_type: BTreeMap<String, LibraryMediaTypeStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_created: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_created: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_added: Option<i64>,
}

/// Rows of each entity table.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntityCounts {
    pub movies: u64,
    pub series: u64,
    /// Episodes known from metadata, with a file or not.
    pub episodes: u64,
    pub people: u64,
    pub tags: u64,
    pub books: u64,
    pub channels: u64,
}

/// Medias with the same content hash.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDuplicates {
    /// Contents stored as more than one media.
    pub groups: u64,
    /// Medias beyond the first of each group.
    pub extra_medias: u64,
    /// Bytes those extra medias take.
    pub extra_size: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFaceCounts {
    /// Medias the face scan has still to process.
    pub pending: u64,
    /// Medias the face scan failed on: it does not try them again.
    pub failed: u64,
    /// Faces found but not attached to a person yet.
    pub unassigned: u64,
}

/// What the library database knows about its content.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LibraryContentStats {
    pub medias: LibraryMediaStats,
    pub entities: LibraryEntityCounts,
    pub duplicates: LibraryDuplicates,
    pub faces: LibraryFaceCounts,
}

impl SqliteLibraryStore {
    pub async fn get_content_stats(&self) -> Result<LibraryContentStats> {
        let res = self
            .connection
            .call(move |conn| {
                let mut medias = LibraryMediaStats::default();
                let mut query = conn.prepare(
                    "SELECT COALESCE(type, 'other'), COUNT(*),
                        CAST(COALESCE(SUM(size), 0) AS INTEGER), CAST(SUM(duration) AS INTEGER)
                     FROM medias GROUP BY COALESCE(type, 'other')",
                )?;
                let rows = query.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        LibraryMediaTypeStats {
                            count: row.get(1)?,
                            size: row.get(2)?,
                            duration: row.get(3)?,
                        },
                    ))
                })?;
                for row in rows {
                    let (kind, stats) = row?;
                    medias.count += stats.count;
                    medias.size += stats.size;
                    medias.by_type.insert(kind, stats);
                }
                (medias.first_created, medias.last_created, medias.last_added) = conn
                    .query_row(
                        "SELECT MIN(created), MAX(created), MAX(added) FROM medias",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )?;

                let (entities, faces) = conn.query_row(
                    "SELECT (SELECT COUNT(*) FROM movies), (SELECT COUNT(*) FROM series),
                        (SELECT COUNT(*) FROM episodes), (SELECT COUNT(*) FROM people),
                        (SELECT COUNT(*) FROM tags), (SELECT COUNT(*) FROM books),
                        (SELECT COUNT(*) FROM channels),
                        (SELECT COUNT(*) FROM medias WHERE face_processed = 0
                            AND (face_recognition_error IS NULL OR face_recognition_error = '')),
                        (SELECT COUNT(*) FROM medias WHERE face_processed = 0
                            AND face_recognition_error IS NOT NULL AND face_recognition_error <> ''),
                        (SELECT COUNT(*) FROM unassigned_faces)",
                    [],
                    |row| {
                        Ok((
                            LibraryEntityCounts {
                                movies: row.get(0)?,
                                series: row.get(1)?,
                                episodes: row.get(2)?,
                                people: row.get(3)?,
                                tags: row.get(4)?,
                                books: row.get(5)?,
                                channels: row.get(6)?,
                            },
                            LibraryFaceCounts {
                                pending: row.get(7)?,
                                failed: row.get(8)?,
                                unassigned: row.get(9)?,
                            },
                        ))
                    },
                )?;

                let duplicates = conn.query_row(
                    "SELECT COUNT(*), COALESCE(SUM(copies - 1), 0),
                        CAST(COALESCE(SUM((copies - 1) * size), 0) AS INTEGER)
                     FROM (SELECT COUNT(*) AS copies, MAX(COALESCE(size, 0)) AS size FROM medias
                           WHERE md5 IS NOT NULL AND md5 <> ''
                           GROUP BY md5 HAVING COUNT(*) > 1)",
                    [],
                    |row| {
                        Ok(LibraryDuplicates {
                            groups: row.get(0)?,
                            extra_medias: row.get(1)?,
                            extra_size: row.get(2)?,
                        })
                    },
                )?;

                Ok(LibraryContentStats {
                    medias,
                    entities,
                    duplicates,
                    faces,
                })
            })
            .await?;
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn content_stats_count_medias_entities_and_duplicates() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        store
            .connection
            .call(|conn| {
                conn.execute_batch(
                    "INSERT INTO medias (id, name, type, size, duration, md5, created, face_processed, face_recognition_error) VALUES
                        ('p1', 'a.jpg', 'photo', 100, NULL, 'same', 1000, 1, NULL),
                        ('p2', 'b.jpg', 'photo', 100, NULL, 'same', 2000, 0, NULL),
                        ('p3', 'c.jpg', 'photo', 100, NULL, 'same', 3000, 0, 'broken'),
                        ('v1', 'd.mp4', 'video', 1000, 30, 'other', 4000, 0, ''),
                        ('x1', 'e.bin', NULL, 7, NULL, NULL, 2500, 1, NULL);
                     INSERT INTO series (id, name) VALUES ('show', 'Show');
                     INSERT INTO episodes (serie_ref, season, number) VALUES ('show', 1, 1), ('show', 1, 2);
                     INSERT INTO tags (id, name) VALUES ('t1', 'Tag');",
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let stats = store.get_content_stats().await.unwrap();

        assert_eq!(stats.medias.count, 5);
        assert_eq!(stats.medias.size, 1307);
        assert_eq!(
            stats.medias.by_type.get("photo"),
            Some(&LibraryMediaTypeStats {
                count: 3,
                size: 300,
                duration: None
            })
        );
        assert_eq!(
            stats.medias.by_type.get("video"),
            Some(&LibraryMediaTypeStats {
                count: 1,
                size: 1000,
                duration: Some(30)
            })
        );
        assert_eq!(stats.medias.by_type.get("other").map(|o| o.count), Some(1));
        assert_eq!(stats.medias.first_created, Some(1000));
        assert_eq!(stats.medias.last_created, Some(4000));
        // The insert trigger stamps `added` with the current time, and `created` when missing.
        assert!(stats.medias.last_added.is_some_and(|added| added > 0));

        assert_eq!(stats.entities.series, 1);
        assert_eq!(stats.entities.episodes, 2);
        assert_eq!(stats.entities.tags, 1);
        assert_eq!(stats.entities.movies, 0);

        assert_eq!(
            stats.duplicates,
            LibraryDuplicates {
                groups: 1,
                extra_medias: 2,
                extra_size: 200
            }
        );
        assert_eq!(
            stats.faces,
            LibraryFaceCounts {
                pending: 2,
                failed: 1,
                unassigned: 0
            }
        );
    }

    #[tokio::test]
    async fn content_stats_of_an_empty_library() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();

        let stats = store.get_content_stats().await.unwrap();

        assert_eq!(stats, LibraryContentStats::default());
    }
}
