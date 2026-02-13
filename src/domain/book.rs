use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ElementAction;

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serie_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chapter: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub airdate: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isbn13: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openlibrary_edition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openlibrary_work_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_books_volume_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asin: Option<String>,
    #[serde(default)]
    pub modified: u64,
    #[serde(default)]
    pub added: u64,
}

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
