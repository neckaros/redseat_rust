use std::{path::PathBuf, sync::Mutex};

use serde::{Deserialize, Serialize};
use strum_macros::EnumString;
use rs_plugin_common_interfaces::{PluginInformation, PluginType};
use extism::Plugin as ExtismPlugin;
use super::{credential::Credential, ElementAction};

#[derive(Debug)]
pub struct PluginWasm {
    pub filename: String,
    pub path: PathBuf,
	pub infos: PluginInformation,
    pub plugin: Mutex<ExtismPlugin>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PluginWithCredential {
    pub plugin: Plugin,
    pub credential: Option<Credential>
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Plugin {
    pub id: String,
	pub name: String,
	pub path: String,
    pub kind: PluginType,
    pub settings: PluginSettings,
    pub libraries: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PluginForAdd {
	pub name: String,
	pub path: String,
    pub kind: PluginType,
    pub settings: PluginSettings,
    pub libraries: Vec<String>,
    pub credential: Option<String>
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
	pub credential: Option<String>,

    pub libraries: Option<Vec<String>>,
    pub add_libraries: Option<Vec<String>>,
    pub remove_libraries: Option<Vec<String>>,
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
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