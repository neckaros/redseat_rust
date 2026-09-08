use super::ElementAction;
pub use rs_plugin_common_interfaces::domain::book::{Book, BookForUpdate};
use rs_plugin_common_interfaces::domain::other_ids::OtherIds;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookWithAction {
    pub action: ElementAction,
    pub book: Book,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BooksMessage {
    pub library: String,
    pub books: Vec<BookWithAction>,
}

pub fn book_enrichment_update(book: &Book, matched: Book) -> BookForUpdate {
    let mut updates = BookForUpdate::default();
    if book.isbn13.is_none() {
        updates.isbn13 = matched.isbn13;
    }
    if book.openlibrary_edition_id.is_none() {
        updates.openlibrary_edition_id = matched.openlibrary_edition_id;
    }
    if book.openlibrary_work_id.is_none() {
        updates.openlibrary_work_id = matched.openlibrary_work_id;
    }
    if book.google_books_volume_id.is_none() {
        updates.google_books_volume_id = matched.google_books_volume_id;
    }
    if book.asin.is_none() {
        updates.asin = matched.asin;
    }
    if book.year.is_none() {
        updates.year = matched.year;
    }
    if book.overview.is_none() {
        updates.overview = matched.overview;
    }
    if book.params.is_none() {
        updates.params = matched.params;
    }
    if book.kind.is_none() {
        updates.kind = matched.kind;
    }
    if book.volume.is_none() {
        updates.volume = matched.volume;
    }
    if book.chapter.is_none() {
        updates.chapter = matched.chapter;
    }
    if book.airdate.is_none() {
        updates.airdate = matched.airdate;
    }
    if book.pages.is_none() {
        updates.pages = matched.pages;
    }
    if book.lang.is_none() {
        updates.lang = matched.lang;
    }
    if book.original.is_none() {
        updates.original = matched.original;
    }
    let ids = super::merge_refresh_ids(book.otherids.as_ref(), matched.otherids.as_ref());
    if ids != book.otherids {
        updates.otherids = ids;
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_refresh_book_enrichment_preserves_existing_data() {
        let stored = Book {
            name: "Book".into(),
            lang: Some("fr".into()),
            otherids: Some(OtherIds::from(vec!["offline:keep".into()])),
            ..Default::default()
        };
        let incoming = Book {
            lang: Some("en".into()),
            original: Some("Original".into()),
            pages: Some(250),
            overview: Some("Description".into()),
            otherids: Some(OtherIds::from(vec!["provider:123".into()])),
            ..Default::default()
        };
        let update = book_enrichment_update(&stored, incoming);
        assert!(update.lang.is_none());
        assert_eq!(update.original.as_deref(), Some("Original"));
        assert_eq!(update.pages, Some(250));
        assert_eq!(update.overview.as_deref(), Some("Description"));
        assert!(update
            .otherids
            .as_ref()
            .unwrap()
            .contains("offline", "keep"));
        assert!(update
            .otherids
            .as_ref()
            .unwrap()
            .contains("provider", "123"));
        assert!(!book_enrichment_update(&stored, Book::default()).has_update());
    }
}
