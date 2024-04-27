use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

use super::ElementAction;


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ServerLibrary {
    pub id: String,
	pub name: String,
	pub source: String,
    pub root: Option<String>,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    pub crypt: Option<bool>,
    pub settings: ServerLibrarySettings,
    pub credentials: Option<String>,
    pub plugin: Option<String>,

    #[serde(default)]
    pub hidden: bool,
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, EnumString, Default)]
#[serde(rename_all = "camelCase")] 
#[strum(serialize_all = "camelCase")]
pub enum LibraryRole {
	Admin,
	Read,
	Write,
    #[default]
	None,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, EnumString, Display)]
#[serde(rename_all = "camelCase")] 
#[strum(serialize_all = "camelCase")]
pub enum LibraryType {
	Photos,
	Shows,
	Movies,
	Iptv,
    #[default]
    Other,
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct ServerLibrarySettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    face_threshold: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ignore_groups: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preduction_model: Option<String>,
}






#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct LibraryMessage {
    pub action: ElementAction,
    pub library: ServerLibrary
}