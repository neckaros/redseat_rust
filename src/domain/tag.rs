use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ElementAction;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")] 
pub struct Tag {
    pub id: String,
	pub name: String,
    pub parent: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub alt: Option<Vec<String>>,
    pub thumb: Option<String>,
    pub params: Option<Value>,
    pub modified: u64,
    pub added: u64,
    pub generated: bool,
    pub path: String,
}

impl Tag {
    pub fn full_path(&self) -> String {
        format!("{}{}", self.path, self.name)
    }
    pub fn childs_path(&self) -> String {
        format!("{}{}/", self.path, self.name)
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct TagMessage {
    pub library: String,
    pub action: ElementAction,
    pub tags: Vec<Tag>
}