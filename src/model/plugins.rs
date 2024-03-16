


use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;


use crate::domain::{backup::Backup, plugin::{Plugin, PluginForAdd, PluginForInsert, PluginForUpdate, PluginType}};

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
            plugin: plugin
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
}
