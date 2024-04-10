use std::{path::PathBuf, sync::Mutex};

use serde::{Deserialize, Serialize};
use strum_macros::EnumString;
use rs_plugin_common_interfaces::{CredentialType, PluginInformation, PluginType};
use extism::Plugin as ExtismPlugin;
use super::{credential::Credential, ElementAction};

#[derive(Debug, Serialize)]
pub struct PluginWasm {
    pub filename: String,
    pub path: PathBuf,
	pub infos: PluginInformation,
    #[serde(skip_serializing)]
    pub plugin: Mutex<ExtismPlugin>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PluginWithCredential {
    pub plugin: Plugin,
    pub credential: Option<Credential>
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct Plugin {
    pub id: String,
	pub name: String,
    pub description: Option<String>,
	pub path: String,
    #[serde(rename = "type")]
    pub kind: PluginType,
    pub settings: PluginSettings,
    pub libraries: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,

    pub installed: bool,
    pub publisher: Option<String>,
    pub version: Option<usize>,
    pub credential_type: Option<CredentialType>,
}

impl From<&PluginWasm> for Plugin {
    fn from(value: &PluginWasm) -> Self {
        Self {
            id: value.filename.clone(),
            name: value.infos.name.clone(),
            path: value.path.to_str().unwrap_or("/").to_owned(),
            kind: value.infos.kind.clone(),
            settings: PluginSettings {..Default::default()},
            libraries: vec![],
            credential: None,
            installed: false,
            publisher: Some(value.infos.publisher.clone()),
            version: Some(value.infos.version),
            description: Some(value.infos.description.clone()),
            credential_type: value.infos.credential_kind.clone()
        }
    }
}

impl From<&PluginWasm> for PluginForAdd {
    fn from(value: &PluginWasm) -> Self {
        Self {
            name: value.infos.name.clone(),
            path: value.filename.to_owned(),
            kind: value.infos.kind.clone(),
            settings: PluginSettings {..Default::default()},
            libraries: vec![],
            credential: None,
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PluginForInstall {
	pub path: String,
    #[serde(rename = "type")]
    pub kind: PluginType,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PluginForAdd {
	pub name: String,
	pub path: String,
    #[serde(rename = "type")]
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
    #[serde(rename = "type")]
    pub kind: Option<PluginType>,
    pub settings: Option<PluginSettings>,
	pub credential: Option<String>,
    #[serde(default)]
	pub remove_credential: bool,

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