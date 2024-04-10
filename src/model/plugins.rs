


use nanoid::nanoid;
use plugin_request_interfaces::RsRequest;
use rs_plugin_common_interfaces::{PluginInformation, PluginType};
use rs_plugin_lookup_interfaces::{RsLookupQuery, RsLookupResult};
use rs_plugin_url_interfaces::RsLink;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::Sender;


use crate::{domain::{backup::Backup, plugin::{Plugin, PluginForAdd, PluginForInsert, PluginForInstall, PluginForUpdate, PluginWasm, PluginWithCredential}, progress::{RsProgress, RsProgressCallback}}, error::RsResult, plugins::sources::SourceRead};

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
		let mut installed_plugins = self.store.get_plugins(query).await?;
        let all_plugins = &self.plugin_manager.plugins;
        for plugin in all_plugins {
            let existing = installed_plugins.iter_mut().find(|r| r.path == plugin.filename);
            if let Some(existing) = existing {
                existing.description = Some(plugin.infos.description.clone());
                existing.credential_type = plugin.infos.credential_kind.clone();
            } else {
                installed_plugins.push(plugin.into());
                
            }
        }
		Ok(installed_plugins)
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

    pub async fn install_plugin(&self, plugin: PluginForInstall, requesting_user: &ConnectedUser) -> Result<Plugin> {
        requesting_user.check_role(&UserRole::Admin)?;
       
        let plugin = self.plugin_manager.plugins.iter().find(|p| p.filename == plugin.path).ok_or(Error::NotFound)?;

        let plugin_for_add:PluginForAdd  = plugin.into();
        
        let plugin = PluginForInsert {
            id: nanoid!(),
            plugin: plugin_for_add
        };
		self.store.add_plugin(plugin.clone()).await?;
        let plugin = self.get_plugin(plugin.id, &requesting_user).await?.ok_or(Error::NotFound)?;
		Ok(plugin)
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


    pub async fn exec_parse(&self, library_id: Option<String>, url: String, requesting_user: &ConnectedUser) -> RsResult<RsLink> {
		if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::UrlParser), ..Default::default() }, &requesting_user).await?;

        Ok(self.plugin_manager.parse(url, plugins).ok_or(Error::NotFound)?)
	}

    pub async fn exec_expand(&self, library_id: Option<String>, link: RsLink, requesting_user: &ConnectedUser) -> RsResult<String> {
		if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::UrlParser), ..Default::default() }, &requesting_user).await?;

        Ok(self.plugin_manager.expand(link, plugins).ok_or(Error::NotFound)?)
	}



    pub async fn exec_request(&self, request: RsRequest, library_id: Option<String>, savable: bool, progress: Option<Sender<RsProgress>>, requesting_user: &ConnectedUser) -> RsResult<SourceRead> {
        if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::Request), ..Default::default() }, &requesting_user).await?;

        Ok(self.plugin_manager.request(request, savable, plugins, progress).await?)
        
    }

    pub async fn exec_lookup(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser) -> RsResult<Vec<RsLookupResult>> {
        if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::Lookup), ..Default::default() }, &requesting_user).await?;

        Ok(self.plugin_manager.lookup(query, plugins).await?)
        
    }
}
