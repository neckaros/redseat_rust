use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, PartialOrd, Default)]
#[serde(rename_all = "snake_case")] 
pub struct RsLinko {
	pub platform: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
}
