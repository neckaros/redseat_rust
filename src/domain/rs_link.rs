use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ElementAction;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")] 
pub struct RsLink {
    pub id: String,
	pub platform: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
}
