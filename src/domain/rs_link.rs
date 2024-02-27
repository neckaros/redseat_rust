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
    pub file: Option<String>,
    pub user: Option<String>,
    pub plugin: Option<String>,
}
