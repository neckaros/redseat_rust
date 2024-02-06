use serde::{Deserialize, Serialize};


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerLibrary {
    pub id: String,
	pub name: String,
	pub source: String,
    pub root: Option<String>,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    pub crypt: Option<bool>,
    pub settings: ServerLibrarySettings
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")] 
pub enum LibraryRole {
	Admin,
	Read,
	Write,
	None,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")] 
pub enum LibraryType {
	Photos,
	Shows,
	Movies,
	Iptv,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct ServerLibrarySettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    face_threshold: Option<f32>,
}