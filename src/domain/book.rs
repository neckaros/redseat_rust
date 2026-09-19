use super::ElementAction;
pub use rs_plugin_common_interfaces::domain::book::{Book, BookForUpdate};
use rs_plugin_common_interfaces::domain::other_ids::OtherIds;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BookWithAction {
    pub action: ElementAction,
    pub book: rs_plugin_common_interfaces::domain::ItemWithRelations<Book>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BooksMessage {
    pub library: String,
    pub books: Vec<BookWithAction>,
}

/// Build the fields that a metadata refresh may update on an existing book.
///
/// Providers often return sparse records, so omitted values are deliberately
/// left untouched. External IDs are merged so a provider cannot discard an ID
/// saved by another source.
pub fn book_metadata_update(book: &Book, incoming: &Book) -> BookForUpdate {
    let mut updates = BookForUpdate::default();

    if !incoming.name.trim().is_empty() && book.name != incoming.name {
        updates.name = Some(incoming.name.clone());
    }
    if book.kind != incoming.kind {
        updates.kind = incoming.kind.clone();
    }
    if book.serie_ref != incoming.serie_ref && incoming.serie_ref.is_some() {
        updates.serie_ref = incoming.serie_ref.clone();
    }
    if book.volume != incoming.volume {
        updates.volume = incoming.volume;
    }
    if book.serie_ref.is_some() && book.chapter != incoming.chapter {
        updates.chapter = incoming.chapter;
    }
    if book.year != incoming.year {
        updates.year = incoming.year;
    }
    if book.airdate != incoming.airdate {
        updates.airdate = incoming.airdate;
    }
    if book.overview != incoming.overview {
        updates.overview = incoming.overview.clone();
    }
    if book.pages != incoming.pages {
        updates.pages = incoming.pages;
    }
    if book.params != incoming.params {
        updates.params = incoming.params.clone();
    }
    if book.lang != incoming.lang {
        updates.lang = incoming.lang.clone();
    }
    if book.original != incoming.original {
        updates.original = incoming.original.clone();
    }
    if book.isbn13 != incoming.isbn13 {
        updates.isbn13 = incoming.isbn13.clone();
    }
    if book.openlibrary_edition_id != incoming.openlibrary_edition_id {
        updates.openlibrary_edition_id = incoming.openlibrary_edition_id.clone();
    }
    if book.openlibrary_work_id != incoming.openlibrary_work_id {
        updates.openlibrary_work_id = incoming.openlibrary_work_id.clone();
    }
    if book.google_books_volume_id != incoming.google_books_volume_id {
        updates.google_books_volume_id = incoming.google_books_volume_id.clone();
    }
    if book.asin != incoming.asin {
        updates.asin = incoming.asin.clone();
    }

    let ids = super::merge_refresh_ids(book.otherids.as_ref(), incoming.otherids.as_ref());
    if ids != book.otherids {
        updates.otherids = ids;
    }

    updates
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
    if book.chapter.is_none() && book.serie_ref.is_some() {
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
    #[test]
    fn metadata_refresh_book_chapter_requires_existing_series() {
        let incoming = Book {
            chapter: Some(3.0),
            overview: Some("Description".into()),
            ..Default::default()
        };
        let standalone = book_enrichment_update(&Book::default(), incoming.clone());
        assert!(standalone.chapter.is_none());
        assert_eq!(standalone.overview.as_deref(), Some("Description"));
        assert!(standalone.has_update());
        let mut linked = Book {
            serie_ref: Some("local-series".into()),
            ..Default::default()
        };
        assert_eq!(
            book_enrichment_update(&linked, incoming.clone()).chapter,
            Some(3.0)
        );
        linked.chapter = Some(1.0);
        assert!(book_enrichment_update(&linked, incoming).chapter.is_none());
    }

    #[test]
    fn metadata_refresh_updates_changed_fields_and_preserves_provider_ids() {
        let stored = Book {
            name: "Old title".into(),
            lang: Some("fr".into()),
            otherids: Some(OtherIds::from(vec!["offline:keep".into()])),
            ..Default::default()
        };
        let incoming = Book {
            name: "New title".into(),
            lang: Some("en".into()),
            overview: Some("Description".into()),
            otherids: Some(OtherIds::from(vec!["provider:123".into()])),
            ..Default::default()
        };

        let update = book_metadata_update(&stored, &incoming);
        assert_eq!(update.name.as_deref(), Some("New title"));
        assert_eq!(update.lang.as_deref(), Some("en"));
        assert_eq!(update.overview.as_deref(), Some("Description"));
        let ids = update.otherids.unwrap();
        assert!(ids.contains("offline", "keep"));
        assert!(ids.contains("provider", "123"));
    }
}
