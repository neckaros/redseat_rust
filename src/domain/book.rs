use rs_plugin_common_interfaces::domain::other_ids::OtherIds;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use rs_plugin_common_interfaces::domain::book::Book;
use super::ElementAction;

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookForUpdate {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub serie_ref: Option<String>,
    pub volume: Option<f64>,
    pub chapter: Option<f64>,
    pub year: Option<u16>,
    pub airdate: Option<i64>,
    pub overview: Option<String>,
    pub pages: Option<u32>,
    pub params: Option<Value>,
    pub lang: Option<String>,
    pub original: Option<String>,
    pub isbn13: Option<String>,
    pub openlibrary_edition_id: Option<String>,
    pub openlibrary_work_id: Option<String>,
    pub google_books_volume_id: Option<String>,
    pub asin: Option<String>,
    pub otherids: Option<OtherIds>,
}

impl BookForUpdate {
    pub fn has_update(&self) -> bool {
        self != &BookForUpdate::default()
    }
}

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
