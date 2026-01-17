use std::{path::PathBuf, sync::Mutex};

use serde::{Deserialize, Serialize};
use strum_macros::EnumString;
use rs_plugin_common_interfaces::{CredentialType, CustomParam, PluginInformation, PluginType};
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
    pub description: String,
	pub path: String,
    pub repo: Option<String>,
    pub repov: Option<String>,
    pub capabilities: Vec<PluginType>,
    pub settings: PluginSettings,
    pub libraries: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,

    pub installed: bool,
    pub publisher: Option<String>,
    pub version: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<CredentialType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<CustomParam>,
}

impl From<&PluginWasm> for Plugin {
    fn from(value: &PluginWasm) -> Self {
        Self {
            id: value.filename.clone(),
            name: value.infos.name.clone(),
            path: value.path.to_str().unwrap_or("/").to_owned(),
            repo: value.infos.repo.clone(),
            repov: None,
            capabilities: value.infos.capabilities.clone(),
            settings: PluginSettings {..Default::default()},
            libraries: vec![],
            credential: None,
            installed: false,
            publisher: Some(value.infos.publisher.clone()),
            version: value.infos.version,
            description: value.infos.description.clone(),
            credential_type: value.infos.credential_kind.clone(),
            params: value.infos.settings.clone(),
        }
    }
}

impl From<&PluginWasm> for PluginForAdd {
    fn from(value: &PluginWasm) -> Self {
        Self {
            name: value.infos.name.clone(),
            path: value.filename.to_owned(),
            repo: value.infos.repo.clone(),
            repov: None,
            credential_type: value.infos.credential_kind.to_owned(),
            description: value.infos.description.to_owned(),
            version: value.infos.version.to_owned(),
            capabilities: value.infos.capabilities.clone(),
            settings: PluginSettings {..Default::default()},
            libraries: vec![],
            credential: None,
        }
    }
}

impl From<PluginInformation> for PluginForUpdate {
    fn from(infos: PluginInformation) -> Self {
        Self {
            credential_type: infos.credential_kind,
            description: Some(infos.description),
            version: Some(infos.version),
            capabilities: Some(infos.capabilities),
            ..Default::default()
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
pub struct PluginRepoAdd {
	pub url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct PluginForAdd {
	pub name: String,
	pub path: String,
	pub repo: Option<String>,
	pub repov: Option<String>,
    pub version: u16,
    pub description: String,
    pub credential_type: Option<CredentialType>,
    pub capabilities: Vec<PluginType>,
    pub settings: PluginSettings,
    pub libraries: Vec<String>,
    pub credential: Option<String>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PluginForInsert {
	pub id: String,
	pub plugin: PluginForAdd,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct PluginForUpdate{
	pub name: Option<String>,
	pub version: Option<u16>,
	pub description: Option<String>,
	pub path: Option<String>,
	pub repo: Option<String>,
	pub repov: Option<String>,
    pub capabilities: Option<Vec<PluginType>>,
    pub settings: Option<PluginSettings>,
    
	pub credential_type: Option<CredentialType>,
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