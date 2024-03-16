use serde::{Deserialize, Serialize};
use strum_macros::EnumString;

use super::ElementAction;


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Plugin {
    pub id: String,
	pub name: String,
	pub path: String,
    pub kind: PluginType,
    pub settings: PluginSettings,
    pub libraries: Vec<String>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PluginForAdd {
	pub name: String,
	pub path: String,
    pub kind: PluginType,
    pub settings: PluginSettings,
    pub libraries: Vec<String>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PluginForInsert {
	pub id: String,
	pub plugin: PluginForAdd,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct PluginForUpdate{
	pub name: Option<String>,
	pub path: Option<String>,
    pub kind: Option<PluginType>,
    pub settings: Option<PluginSettings>,

    pub libraries: Option<Vec<String>>,
    pub add_libraries: Option<Vec<String>>,
    pub remove_libraries: Option<Vec<String>>,
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display,EnumString)]
#[serde(rename_all = "camelCase")] 
#[strum(serialize_all = "camelCase")]
pub enum PluginType {
	ImageClassification,
    UrlParser,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct PluginSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bgr: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalize: Option<bool>,
}






#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct LibraryMessage {
    pub action: ElementAction,
    pub plugin: Plugin
}