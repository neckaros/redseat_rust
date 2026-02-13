use rs_plugin_common_interfaces::domain::rs_ids::RsIds;
use rusqlite::{params, OptionalExtension, Row};

use crate::{
    domain::book::Book,
    model::{
        books::BookQuery,
        store::sql::{
            OrderBuilder, QueryBuilder, QueryWhereType, RsQueryBuilder, SqlOrder, SqlWhereType,
        },
        Error,
    },
};

use super::{Result, SqliteLibraryStore};

impl SqliteLibraryStore {
    fn row_to_book(row: &Row) -> rusqlite::Result<Book> {
        Ok(Book {
            id: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            serie_ref: row.get(3)?,
            volume: row.get(4)?,
            chapter: row.get(5)?,
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
            modified: row.get(18)?,
            added: row.get(19)?,
        })
    }

    pub async fn get_books(&self, query: BookQuery) -> Result<Vec<Book>> {
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
                    id, name, type, serie_ref, volume, chapter, year, airdate, overview, pages, params, lang, original,
                    isbn13, openlibrary_edition_id, openlibrary_work_id, google_books_volume_id, asin, modified, added
                    FROM books {}{}",
                    where_query.format(),
                    where_query.format_order()
                ))?;
                let rows = statement.query_map(where_query.values(), Self::row_to_book)?;
                let values = rows.collect::<std::result::Result<Vec<Book>, rusqlite::Error>>()?;
                Ok(values)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_book(&self, book_id: &str) -> Result<Option<Book>> {
        let book_id = book_id.to_string();
        let row = self
            .connection
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT
                    id, name, type, serie_ref, volume, chapter, year, airdate, overview, pages, params, lang, original,
                    isbn13, openlibrary_edition_id, openlibrary_work_id, google_books_volume_id, asin, modified, added
                    FROM books WHERE id = ?",
                )?;
                let row = statement
                    .query_row([book_id], Self::row_to_book)
                    .optional()?;
                Ok(row)
            })
            .await?;
        Ok(row)
    }

    pub async fn get_book_by_external_id(&self, ids: RsIds) -> Result<Option<Book>> {
        let row = self
            .connection
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT
                    id, name, type, serie_ref, volume, chapter, year, airdate, overview, pages, params, lang, original,
                    isbn13, openlibrary_edition_id, openlibrary_work_id, google_books_volume_id, asin, modified, added
                    FROM books
                    WHERE id = ? or isbn13 = ? or openlibrary_edition_id = ? or openlibrary_work_id = ? or google_books_volume_id = ? or asin = ?",
                )?;
                let row = statement
                    .query_row(
                        params![
                            ids.redseat.unwrap_or("zz".to_string()),
                            ids.isbn13.unwrap_or("zz".to_string()),
                            ids.openlibrary_edition_id.unwrap_or("zz".to_string()),
                            ids.openlibrary_work_id.unwrap_or("zz".to_string()),
                            ids.google_books_volume_id.unwrap_or("zz".to_string()),
                            ids.asin.unwrap_or("zz".to_string()),
                        ],
                        Self::row_to_book,
                    )
                    .optional()?;
                Ok(row)
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
                        isbn13, openlibrary_edition_id, openlibrary_work_id, google_books_volume_id, asin
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                        book.asin
                    ],
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
                where_query.add_where(QueryWhereType::Equal("id", &book_id));
                if where_query.columns_update.is_empty() {
                    return Ok(());
                }
                let sql = format!(
                    "UPDATE books SET {} {}",
                    where_query.format_update(),
                    where_query.format()
                );
                conn.execute(&sql, where_query.values())?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn remove_book(&self, book_id: String) -> Result<()> {
        self.connection
            .call(move |conn| {
                conn.execute("DELETE FROM books WHERE id = ?", [book_id.clone()])?;
                conn.execute(
                    "INSERT INTO deleted (id, type) VALUES (?, ?)",
                    [&book_id, "book"],
                )?;
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
        model::books::BookQuery,
    };
    use rs_plugin_common_interfaces::domain::rs_ids::RsIds;

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
        assert_eq!(fetched.volume, Some(12.0));
        assert_eq!(fetched.chapter, Some(108.0));

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
        assert_eq!(updated.isbn13.as_deref(), Some("9783161484100"));
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
        assert!(fetched.isbn13.is_none());
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
}
