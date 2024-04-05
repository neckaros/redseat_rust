


use nanoid::nanoid;
use plugin_request_interfaces::RsRequest;
use rs_plugin_common_interfaces::{PluginInformation, PluginType};
use rs_plugin_lookup_interfaces::{RsLookupQuery, RsLookupResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::Sender;


use crate::{domain::{backup::Backup, plugin::{Plugin, PluginForAdd, PluginForInsert, PluginForUpdate, PluginWithCredential}, progress::{RsProgress, RsProgressCallback}}, error::RsResult, plugins::sources::SourceRead};

use super::{error::{Error, Result}, users::{ConnectedUser, UserRole}, ModelController};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PluginQuery {
    pub kind: Option<PluginType>,
    pub name: Option<String>,
    pub library: Option<String>,
}


impl ModelController {

	pub async fn get_plugins(&self, query: PluginQuery, requesting_user: &ConnectedUser) -> Result<Vec<Plugin>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let credentials = self.store.get_plugins(query).await?;
		Ok(credentials)
	}

    pub async fn get_plugins_with_credential(&self, query: PluginQuery, requesting_user: &ConnectedUser) -> Result<impl Iterator<Item = PluginWithCredential>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let plugins = self.store.get_plugins(query).await?.into_iter();
		let credentials = self.store.get_credentials().await?;
        let iter = plugins.map(move |p| {
            let credential = credentials.iter().find(|c| Some(&c.id) == p.credential.as_ref()).cloned();
            PluginWithCredential { plugin: p.clone(), credential }
        });
		Ok(iter)
	}

    pub async fn get_wasm(&self, _query: PluginQuery, requesting_user: &ConnectedUser) -> Result<Vec<&PluginInformation>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let wasm: Vec<&PluginInformation> = self.plugin_manager.plugins.iter().map(|p| &p.infos).collect();
		Ok(wasm)
	}


    pub async fn get_plugin(&self, plugin_id: String, requesting_user: &ConnectedUser) -> Result<Option<Plugin>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let credential = self.store.get_plugin(&plugin_id).await?;
		Ok(credential)
	}

    pub async fn update_plugin(&self, plugin_id: &str, update: PluginForUpdate, requesting_user: &ConnectedUser) -> Result<Plugin> {
        requesting_user.check_role(&UserRole::Admin)?;
		self.store.update_plugin(plugin_id, update).await?;
        let plugin = self.store.get_plugin(plugin_id).await?;
        if let Some(plugin) = plugin { 
            Ok(plugin)
        } else {
            Err(Error::NotFound)
        }
	}


    pub async fn add_plugin(&self, plugin: PluginForAdd, requesting_user: &ConnectedUser) -> Result<Plugin> {
        requesting_user.check_role(&UserRole::Admin)?;
        let plugin = PluginForInsert {
            id: nanoid!(),
            plugin
        };
		self.store.add_plugin(plugin.clone()).await?;
        let plugin = self.get_plugin(plugin.id, &requesting_user).await?.ok_or(Error::NotFound)?;
		Ok(plugin)
	}


    pub async fn remove_plugin(&self, plugin_id: &str, requesting_user: &ConnectedUser) -> Result<Plugin> {
        requesting_user.check_role(&UserRole::Admin)?;
        let credential = self.store.get_plugin(&plugin_id).await?;
        if let Some(credential) = credential { 
            self.store.remove_plugin(plugin_id.to_string()).await?;
            Ok(credential)
        } else {
            Err(Error::NotFound)
        }
	}
    pub async fn exec_request(&self, request: RsRequest, library_id: Option<String>, progress: Option<Sender<RsProgress>>, requesting_user: &ConnectedUser) -> RsResult<SourceRead> {
        if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Read)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::Request), ..Default::default() }, &requesting_user).await?;

        Ok(self.plugin_manager.request(request, plugins, progress).await?)
        
    }

    
    pub async fn exec_lookup(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser) -> RsResult<Vec<RsLookupResult>> {
        if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Read)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::Lookup), ..Default::default() }, &requesting_user).await?;
        
        Ok(self.plugin_manager.lookup(query, plugins).await?)
        
    }
}
