use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ElementAction;
use rs_plugin_common_interfaces::url::RsLink;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
	pub name: String,
    pub socials: Option<Vec<RsLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portrait: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday: Option<u64>,
    pub modified: u64,
    pub added: u64,
    pub posterv: u32,
    #[serde(default)]
    pub generated: bool,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct PeopleMessage {
    pub library: String,
    pub people: Vec<PersonWithAction>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct PersonWithAction {
    pub action: ElementAction,
    pub person: Person
}
