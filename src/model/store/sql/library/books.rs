use rs_plugin_common_interfaces::domain::{
    media::{FileEpisode, MediaItemReference}, other_ids::OtherIds, rs_ids::RsIds, ItemWithRelations, Relations,
};
use rusqlite::{params, OptionalExtension, Row};

use crate::{
    domain::book::Book,
    model::{
        books::BookQuery,
        store::{
            from_comma_separated_optional,
            sql::{
                OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder, SqlOrder, SqlWhereType,
            },
        },
        Error,
    },
    tools::log::{log_info, LogServiceType},
};

use super::{Result, SqliteLibraryStore};

impl SqliteLibraryStore {
    fn row_to_book(row: &Row) -> rusqlite::Result<ItemWithRelations<Book>> {
        let serie_ref: Option<String> = row.get(3)?;
        let volume: Option<f64> = row.get(4)?;
        let chapter: Option<f64> = row.get(5)?;
        let tags_raw: Option<String> = row.get(21)?;
        let people_raw: Option<String> = row.get(22)?;

        let tags = from_comma_separated_optional::<MediaItemReference>(tags_raw);
        let people = from_comma_separated_optional::<MediaItemReference>(people_raw);

        let has_relations = serie_ref.is_some() || tags.is_some() || people.is_some();
        let relations = if has_relations {
            Some(Relations {
                series: serie_ref.as_ref().map(|id| {
                    vec![FileEpisode {
                        id: id.clone(),
                        season: volume.map(|v| v as u32),
                        episode: chapter.map(|c| c as u32),
                        episode_to: None,
                    }]
                }),
                tags,
                people,
                ..Default::default()
            })
        } else {
            None
        };

        Ok(ItemWithRelations {
            item: Book {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                serie_ref,
                volume,
                chapter,
                year: row.get(6)?,
                airdate: row.get(7)?,
                overview: row.get(8)?,
                pages: row.get(9)?,
                params: row.get(10)?,
                lang: row.get(11)?,
                original: row.get(12)?,
                isbn13: row.get(13)?,
                openlibrary_edition_id: row.get(14)?,
                openlibrary_work_id: row.get(15)?,
                google_books_volume_id: row.get(16)?,
                asin: row.get(17)?,
                otherids: row.get(18)?,
                modified: row.get(19)?,
                added: row.get(20)?,
            },
            relations,
        })
    }

    pub async fn get_books(&self, query: BookQuery) -> Result<Vec<ItemWithRelations<Book>>> {
        let row = self
            .connection
            .call(move |conn| {
                let mut where_query = RsQueryBuilder::new();
                if let Some(after) = query.after {
                    where_query.add_where(SqlWhereType::After("modified".to_string(), Box::new(after)));
                }
                if let Some(name) = query.name {
                    where_query.add_where(SqlWhereType::Like("name".to_string(), Box::new(format!("%{}%", name))));
                }
                if let Some(serie_ref) = query.serie_ref {
                    where_query.add_where(SqlWhereType::Equal("serie_ref".to_string(), Box::new(serie_ref)));
                }
                if let Some(isbn13) = query.isbn13 {
                    where_query.add_where(SqlWhereType::Equal("isbn13".to_string(), Box::new(isbn13)));
                }
                if let Some(openlibrary_edition_id) = query.openlibrary_edition_id {
                    where_query.add_where(SqlWhereType::Equal(
                        "openlibrary_edition_id".to_string(),
                        Box::new(openlibrary_edition_id),
                    ));
                }
                if let Some(openlibrary_work_id) = query.openlibrary_work_id {
                    where_query.add_where(SqlWhereType::Equal(
                        "openlibrary_work_id".to_string(),
                        Box::new(openlibrary_work_id),
                    ));
                }
                if let Some(google_books_volume_id) = query.google_books_volume_id {
                    where_query.add_where(SqlWhereType::Equal(
                        "google_books_volume_id".to_string(),
                        Box::new(google_books_volume_id),
                    ));
                }
                if let Some(asin) = query.asin {
                    where_query.add_where(SqlWhereType::Equal("asin".to_string(), Box::new(asin)));
                }
                where_query.add_oder(OrderBuilder::new(query.sort.to_string(), query.order));
                let mut statement = conn.prepare(&format!(
                    "SELECT
                    b.id, b.name, b.type, b.serie_ref, b.volume, b.chapter, b.year, b.airdate, b.overview, b.pages, b.params, b.lang, b.original,
                    b.isbn13, b.openlibrary_edition_id, b.openlibrary_work_id, b.google_books_volume_id, b.asin, b.otherids, b.modified, b.added,
                    (SELECT GROUP_CONCAT(tag_ref || '|' || IFNULL(confidence, 100)) FROM book_tag_mapping WHERE book_ref = b.id) AS tags,
                    (SELECT GROUP_CONCAT(people_ref) FROM book_people_mapping WHERE book_ref = b.id) AS people
                    FROM books b {}{}",
                    where_query.format(),
                    where_query.format_order()
                ))?;
                let rows = statement.query_map(where_query.values(), Self::row_to_book)?;
                let values = rows.collect::<std::result::Result<Vec<ItemWithRelations<Book>>, rusqlite::Error>>()?;
                Ok(values)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_book(&self, book_id: &str) -> Result<Option<ItemWithRelations<Book>>> {
        let book_id = book_id.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT
                    b.id, b.name, b.type, b.serie_ref, b.volume, b.chapter, b.year, b.airdate, b.overview, b.pages, b.params, b.lang, b.original,
                    b.isbn13, b.openlibrary_edition_id, b.openlibrary_work_id, b.google_books_volume_id, b.asin, b.otherids, b.modified, b.added,
                    (SELECT GROUP_CONCAT(tag_ref || '|' || IFNULL(confidence, 100)) FROM book_tag_mapping WHERE book_ref = b.id) AS tags,
                    (SELECT GROUP_CONCAT(people_ref) FROM book_people_mapping WHERE book_ref = b.id) AS people
                    FROM books b WHERE b.id = ?",
                )?;
                let row = statement
                    .query_row([book_id], Self::row_to_book)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_book_by_external_id(&self, ids: RsIds) -> Result<Option<ItemWithRelations<Book>>> {
        let row = self
            .connection
            .call(move |conn| {
                let mut direct_statement = conn.prepare(
                    "SELECT
                    b.id, b.name, b.type, b.serie_ref, b.volume, b.chapter, b.year, b.airdate, b.overview, b.pages, b.params, b.lang, b.original,
                    b.isbn13, b.openlibrary_edition_id, b.openlibrary_work_id, b.google_books_volume_id, b.asin, b.otherids, b.modified, b.added,
                    (SELECT GROUP_CONCAT(tag_ref || '|' || IFNULL(confidence, 100)) FROM book_tag_mapping WHERE book_ref = b.id) AS tags,
                    (SELECT GROUP_CONCAT(people_ref) FROM book_people_mapping WHERE book_ref = b.id) AS people
                    FROM books b
                    WHERE b.id = ? or b.isbn13 = ? or b.openlibrary_edition_id = ? or b.openlibrary_work_id = ? or b.google_books_volume_id = ? or b.asin = ?",
                )?;
                let direct_row = direct_statement
                    .query_row(
                        params![
                            ids.redseat.clone().unwrap_or("zz".to_string()),
                            ids.isbn13.clone().unwrap_or("zz".to_string()),
                            ids.openlibrary_edition_id.clone().unwrap_or("zz".to_string()),
                            ids.openlibrary_work_id.clone().unwrap_or("zz".to_string()),
                            ids.google_books_volume_id.clone().unwrap_or("zz".to_string()),
                            ids.asin.clone().unwrap_or("zz".to_string()),
                        ],
                        Self::row_to_book,
                    )
                    .optional()?;

                let volume_matches = |iwr: &ItemWithRelations<Book>| {
                    let book = &iwr.item;
                    ids.volume.map(|v| Some(v) == book.volume).unwrap_or(true)
                        && ids.chapter.map(|c| Some(c) == book.chapter).unwrap_or(true)
                };

                if let Some(book) = direct_row {
                    if volume_matches(&book) {
                        return Ok(Some(book));
                    }
                }

                let has_series_identity = ids.openlibrary_work_id.is_some()
                    || ids.anilist_manga_id.is_some()
                    || ids.mangadex_manga_uuid.is_some()
                    || ids.myanimelist_manga_id.is_some();
                if !has_series_identity {
                    return Ok(None);
                }

                let mut by_series_statement = conn.prepare(
                    "SELECT
                    b.id, b.name, b.type, b.serie_ref, b.volume, b.chapter, b.year, b.airdate, b.overview, b.pages, b.params, b.lang, b.original,
                    b.isbn13, b.openlibrary_edition_id, b.openlibrary_work_id, b.google_books_volume_id, b.asin, b.otherids, b.modified, b.added,
                    (SELECT GROUP_CONCAT(tag_ref || '|' || IFNULL(confidence, 100)) FROM book_tag_mapping WHERE book_ref = b.id) AS tags,
                    (SELECT GROUP_CONCAT(people_ref) FROM book_people_mapping WHERE book_ref = b.id) AS people
                    FROM books b
                    INNER JOIN series s ON s.id = b.serie_ref
                    WHERE (
                        (?1 IS NOT NULL AND s.openlibrary_work_id = ?1)
                        OR (?2 IS NOT NULL AND s.anilist_manga_id = ?2)
                        OR (?3 IS NOT NULL AND s.mangadex_manga_uuid = ?3)
                        OR (?4 IS NOT NULL AND s.myanimelist_manga_id = ?4)
                    )
                    AND (?5 IS NULL OR b.volume = ?5)
                    AND (?6 IS NULL OR b.chapter = ?6)
                    ORDER BY b.added DESC
                    LIMIT 1",
                )?;

                let by_series_row = by_series_statement
                    .query_row(
                        params![
                            ids.openlibrary_work_id.clone(),
                            ids.anilist_manga_id,
                            ids.mangadex_manga_uuid.clone(),
                            ids.myanimelist_manga_id,
                            ids.volume,
                            ids.chapter,
                        ],
                        Self::row_to_book,
                    )
                    .optional()?;

                Ok(by_series_row)
            })
            .await?;
        Ok(row)
    }

    pub async fn add_book(&self, book: Book) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO books (
                        id, name, type, serie_ref, volume, chapter, year, airdate, overview, pages, params, lang, original,
                        isbn13, openlibrary_edition_id, openlibrary_work_id, google_books_volume_id, asin, otherids
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        book.id,
                        book.name,
                        book.kind,
                        book.serie_ref,
                        book.volume,
                        book.chapter,
                        book.year,
                        book.airdate,
                        book.overview,
                        book.pages,
                        book.params,
                        book.lang,
                        book.original,
                        book.isbn13,
                        book.openlibrary_edition_id,
                        book.openlibrary_work_id,
                        book.google_books_volume_id,
                        book.asin,
                        book.otherids
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn add_book_tag(&self, book_id: &str, tag_id: &str, confidence: Option<i32>) -> Result<()> {
        let book_id = book_id.to_string();
        let tag_id = tag_id.to_string();
        self.connection
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO book_tag_mapping (book_ref, tag_ref, confidence) VALUES (?, ?, ?)",
                    params![book_id, tag_id, confidence],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn remove_book_tag(&self, book_id: &str, tag_id: &str) -> Result<()> {
        let book_id = book_id.to_string();
        let tag_id = tag_id.to_string();
        self.connection
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM book_tag_mapping WHERE book_ref = ? AND tag_ref = ?",
                    params![book_id, tag_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn add_book_person(&self, book_id: &str, person_id: &str, confidence: Option<i32>) -> Result<()> {
        let book_id = book_id.to_string();
        let person_id = person_id.to_string();
        self.connection
            .call(move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO book_people_mapping (book_ref, people_ref, confidence) VALUES (?, ?, ?)",
                    params![book_id, person_id, confidence],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn remove_book_person(&self, book_id: &str, person_id: &str) -> Result<()> {
        let book_id = book_id.to_string();
        let person_id = person_id.to_string();
        self.connection
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM book_people_mapping WHERE book_ref = ? AND people_ref = ?",
                    params![book_id, person_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn update_book(
        &self,
        book_id: &str,
        update: crate::domain::book::BookForUpdate,
    ) -> Result<()> {
        let book_id = book_id.to_string();
        self.connection
            .call(move |conn| {
                let mut where_query = QueryBuilder::new();
                where_query.add_update(&update.name, "name");
                where_query.add_update(&update.kind, "type");
                where_query.add_update(&update.serie_ref, "serie_ref");
                where_query.add_update(&update.volume, "volume");
                where_query.add_update(&update.chapter, "chapter");
                where_query.add_update(&update.year, "year");
                where_query.add_update(&update.airdate, "airdate");
                where_query.add_update(&update.overview, "overview");
                where_query.add_update(&update.pages, "pages");
                where_query.add_update(&update.params, "params");
                where_query.add_update(&update.lang, "lang");
                where_query.add_update(&update.original, "original");
                where_query.add_update(&update.isbn13, "isbn13");
                where_query.add_update(&update.openlibrary_edition_id, "openlibrary_edition_id");
                where_query.add_update(&update.openlibrary_work_id, "openlibrary_work_id");
                where_query.add_update(&update.google_books_volume_id, "google_books_volume_id");
                where_query.add_update(&update.asin, "asin");
                where_query.add_update(&update.otherids, "otherids");
                where_query.add_where(QueryWhereType::Equal("id", &book_id));
                if !where_query.columns_update.is_empty() {
                    let sql = format!(
                        "UPDATE books SET {} {}",
                        where_query.format_update(),
                        where_query.format()
                    );
                    conn.execute(&sql, where_query.values())?;
                }

                if let Some(add_tags) = update.add_tags {
                    for tag in add_tags {
                        let r = conn.execute(
                            "INSERT OR REPLACE INTO book_tag_mapping (book_ref, tag_ref, confidence) VALUES (?, ?, ?)",
                            params![book_id, tag.id, tag.conf],
                        );
                        if let Err(error) = r {
                            log_info(LogServiceType::Source, format!("unable to add book tag {:?}: {:?}", tag.id, error));
                        }
                    }
                }
                if let Some(remove_tags) = update.remove_tags {
                    for tag_id in remove_tags {
                        conn.execute(
                            "DELETE FROM book_tag_mapping WHERE book_ref = ? AND tag_ref = ?",
                            params![book_id, tag_id],
                        )?;
                    }
                }

                if let Some(add_people) = update.add_people {
                    for person in add_people {
                        let r = conn.execute(
                            "INSERT OR REPLACE INTO book_people_mapping (book_ref, people_ref, confidence) VALUES (?, ?, ?)",
                            params![book_id, person.id, person.conf],
                        );
                        if let Err(error) = r {
                            log_info(LogServiceType::Source, format!("unable to add book person {:?}: {:?}", person.id, error));
                        }
                    }
                }
                if let Some(remove_people) = update.remove_people {
                    for person_id in remove_people {
                        conn.execute(
                            "DELETE FROM book_people_mapping WHERE book_ref = ? AND people_ref = ?",
                            params![book_id, person_id],
                        )?;
                    }
                }

                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn remove_book(&self, book_id: String) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute("DELETE FROM books WHERE id = ?", [book_id.clone()])?;
                conn.execute("DELETE FROM book_tag_mapping  WHERE book_ref = ?", &[&book_id])?;
                conn.execute("DELETE FROM book_people_mapping  WHERE book_ref = ?", &[&book_id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteLibraryStore;
    use crate::{
        domain::book::{Book, BookForUpdate},
        domain::serie::Serie,
        model::books::BookQuery,
    };
    use rs_plugin_common_interfaces::domain::{media::MediaItemReference, rs_ids::RsIds};

    #[tokio::test]
    async fn books_crud_roundtrip_with_volume_chapter_modes() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();

        let volume = Book {
            id: "book-v".to_string(),
            name: "Volume 12".to_string(),
            serie_ref: Some("serie-1".to_string()),
            volume: Some(12.0),
            chapter: None,
            ..Default::default()
        };
        store.add_book(volume.clone()).await.unwrap();

        let chapter = Book {
            id: "book-c".to_string(),
            name: "Chapter 1050".to_string(),
            serie_ref: Some("serie-1".to_string()),
            volume: None,
            chapter: Some(1050.0),
            ..Default::default()
        };
        store.add_book(chapter.clone()).await.unwrap();

        let hybrid = Book {
            id: "book-h".to_string(),
            name: "Volume 12 Chapter 108".to_string(),
            serie_ref: Some("serie-1".to_string()),
            volume: Some(12.0),
            chapter: Some(108.0),
            ..Default::default()
        };
        store.add_book(hybrid.clone()).await.unwrap();

        let fetched = store.get_book("book-h").await.unwrap().unwrap();
        assert_eq!(fetched.item.volume, Some(12.0));
        assert_eq!(fetched.item.chapter, Some(108.0));
        // Relations should include series with volume=season, chapter=episode
        let series = fetched.relations.unwrap().series.unwrap();
        assert_eq!(series[0].id, "serie-1");
        assert_eq!(series[0].season, Some(12));
        assert_eq!(series[0].episode, Some(108));

        let all = store.get_books(BookQuery::default()).await.unwrap();
        assert_eq!(all.len(), 3);

        store
            .update_book(
                "book-h",
                BookForUpdate {
                    isbn13: Some("9783161484100".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = store.get_book("book-h").await.unwrap().unwrap();
        assert_eq!(updated.item.isbn13.as_deref(), Some("9783161484100"));
    }

    #[tokio::test]
    async fn books_chapter_requires_serie_ref() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        let invalid = Book {
            id: "book-invalid".to_string(),
            name: "Chapter only".to_string(),
            chapter: Some(1.0),
            ..Default::default()
        };
        let result = store.add_book(invalid).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn books_no_isbn_required() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        store
            .add_book(Book {
                id: "book-no-isbn".to_string(),
                name: "No ISBN".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();
        let fetched = store.get_book("book-no-isbn").await.unwrap().unwrap();
        assert!(fetched.item.isbn13.is_none());
        assert!(fetched.relations.is_none());
    }

    #[tokio::test]
    async fn books_external_id_lookup_uses_book_ids_only() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        store
            .add_book(Book {
                id: "book-ids".to_string(),
                name: "IDs".to_string(),
                isbn13: Some("9783161484100".to_string()),
                openlibrary_edition_id: Some("OL7353617M".to_string()),
                openlibrary_work_id: Some("OL45883W".to_string()),
                google_books_volume_id: Some("zyTCAlFPjgYC".to_string()),
                asin: Some("B00TESTASIN".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        let found = store
            .get_book_by_external_id(RsIds {
                isbn13: Some("9783161484100".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(found.is_some());

        let not_found = store
            .get_book_by_external_id(RsIds {
                anilist_manga_id: Some(12345),
                mangadex_manga_uuid: Some("mangadex-id".to_string()),
                myanimelist_manga_id: Some(67890),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn books_external_id_lookup_with_series_and_volume_chapter() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();

        store
            .add_serie(Serie {
                id: "serie-manga".to_string(),
                name: "Manga".to_string(),
                anilist_manga_id: Some(4242),
                ..Default::default()
            })
            .await
            .unwrap();

        store
            .add_book(Book {
                id: "book-manga-v1".to_string(),
                name: "Vol 1".to_string(),
                serie_ref: Some("serie-manga".to_string()),
                volume: Some(1.0),
                chapter: Some(1.0),
                ..Default::default()
            })
            .await
            .unwrap();
        store
            .add_book(Book {
                id: "book-manga-v2".to_string(),
                name: "Vol 2".to_string(),
                serie_ref: Some("serie-manga".to_string()),
                volume: Some(2.0),
                chapter: Some(10.0),
                ..Default::default()
            })
            .await
            .unwrap();

        let found = store
            .get_book_by_external_id(RsIds {
                anilist_manga_id: Some(4242),
                volume: Some(2.0),
                chapter: Some(10.0),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(found.map(|b| b.item.id), Some("book-manga-v2".to_string()));
    }

    #[tokio::test]
    async fn books_tags_and_people_crud() {
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();

        store
            .add_book(Book {
                id: "book-rel".to_string(),
                name: "Relations Book".to_string(),
                ..Default::default()
            })
            .await
            .unwrap();

        // Add tags
        store.add_book_tag("book-rel", "tag-1", Some(100)).await.unwrap();
        store.add_book_tag("book-rel", "tag-2", None).await.unwrap();

        // Add people
        store.add_book_person("book-rel", "person-1", Some(90)).await.unwrap();

        let fetched = store.get_book("book-rel").await.unwrap().unwrap();
        let relations = fetched.relations.unwrap();
        let tags = relations.tags.unwrap();
        let people = relations.people.unwrap();

        assert_eq!(tags.len(), 2);
        assert!(tags.iter().any(|t| t.id == "tag-1"));
        assert!(tags.iter().any(|t| t.id == "tag-2"));
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].id, "person-1");

        // Remove tag
        store.remove_book_tag("book-rel", "tag-1").await.unwrap();
        let fetched2 = store.get_book("book-rel").await.unwrap().unwrap();
        let tags2 = fetched2.relations.unwrap().tags.unwrap();
        assert_eq!(tags2.len(), 1);
        assert_eq!(tags2[0].id, "tag-2");

        // Update with add/remove via BookForUpdate
        store
            .update_book(
                "book-rel",
                crate::domain::book::BookForUpdate {
                    add_tags: Some(vec![MediaItemReference { id: "tag-3".to_string(), conf: None }]),
                    remove_tags: Some(vec!["tag-2".to_string()]),
                    remove_people: Some(vec!["person-1".to_string()]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let fetched3 = store.get_book("book-rel").await.unwrap().unwrap();
        let relations3 = fetched3.relations.unwrap();
        let tags3 = relations3.tags.unwrap();
        assert_eq!(tags3.len(), 1);
        assert_eq!(tags3[0].id, "tag-3");
        assert!(relations3.people.is_none());
    }
}
