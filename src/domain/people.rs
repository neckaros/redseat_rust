use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{rs_link::RsLink, ElementAction};


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")] 
pub struct Person {
    pub id: String,
	pub name: String,
    pub socials: Option<Vec<RsLink>>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub alt: Option<Vec<String>>,
    pub portrait: Option<String>,
    pub params: Option<Value>,
    pub birthday: u64,
    pub modified: u64,
    pub added: u64
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct PeopleMessage {
    pub library: String,
    pub action: ElementAction,
    pub people: Vec<Person>
}