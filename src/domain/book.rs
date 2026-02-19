use rs_plugin_common_interfaces::domain::other_ids::OtherIds;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use rs_plugin_common_interfaces::domain::book::{Book, BookForUpdate};
use super::ElementAction;

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
