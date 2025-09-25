


use std::collections::HashMap;

use nanoid::nanoid;
use rs_plugin_common_interfaces::{lookup::{RsLookupQuery, RsLookupSourceResult}, request::RsRequest, url::{RsLink, RsLinkType}, PluginCredential, PluginInformation, PluginType, RsPluginRequest};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{fs::File, io::{copy, BufWriter}, sync::mpsc::Sender};


use crate::{domain::{backup::Backup, library::LibraryRole, plugin::{Plugin, PluginForAdd, PluginForInsert, PluginForInstall, PluginForUpdate, PluginWasm, PluginWithCredential}, progress::{RsProgress, RsProgressCallback}}, error::RsResult, plugins::{get_plugin_fodler, sources::{error::SourcesError, AsyncReadPinBox, SourceRead}}, tools::video_tools::ytdl::YydlContext};

use super::{error::{Error, Result}, users::{ConnectedUser, UserRole}, ModelController};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PluginQuery {
    pub kind: Option<PluginType>,
    pub name: Option<String>,
    pub library: Option<String>,
}


impl ModelController {

	pub async fn get_all_plugins(&self, query: PluginQuery, requesting_user: &ConnectedUser) -> Result<Vec<Plugin>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let mut installed_plugins = self.store.get_plugins(query).await?;
        let all_plugins = &self.plugin_manager.plugins;
        for plugin in all_plugins.read().await.iter() {
            let existing = installed_plugins.iter_mut().find(|r| r.path == plugin.filename);
            if let Some(existing) = existing {
                existing.description = plugin.infos.description.clone();
                existing.credential_type = plugin.infos.credential_kind.clone();
            } else {
                installed_plugins.push(plugin.into());
                
            }
        }
		Ok(installed_plugins)
	}


    pub async fn get_plugins(&self, query: PluginQuery, requesting_user: &ConnectedUser) -> Result<Vec<Plugin>> {
        if let Some(library_id) = &query.library {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Write)?;
        } else {
            requesting_user.check_role(&UserRole::Write)?;
        }

		let plugins = self.store.get_plugins(query).await?;
		
		Ok(plugins)
	}

    async fn get_plugins_with_credential(&self, query: PluginQuery) -> Result<impl Iterator<Item = PluginWithCredential>> {
		let plugins = self.store.get_plugins(query).await?.into_iter();
		let credentials = self.store.get_credentials().await?;
        let iter = plugins.map(move |p| {
            let credential = credentials.iter().find(|c| Some(&c.id) == p.credential.as_ref()).cloned();
            PluginWithCredential { plugin: p.clone(), credential }
        });
		Ok(iter)
	}

    pub async fn get_plugin(&self, plugin_id: String, requesting_user: &ConnectedUser) -> RsResult<Plugin> {
        requesting_user.check_role(&UserRole::Admin)?;
		let credential = self.store.get_plugin(&plugin_id).await?.ok_or(SourcesError::UnableToFindPlugin(plugin_id.to_string(), "get_plugin".to_string()))?;
		Ok(credential)
	}

    pub async fn reload_plugins(&self, requesting_user: &ConnectedUser) -> RsResult<()> {
        requesting_user.check_role(&UserRole::Admin)?;
        self.plugin_manager.reload().await?;
		Ok(())
	}

    pub async fn reload_plugin(&self, plugin_id: String, requesting_user: &ConnectedUser) -> RsResult<Plugin> {
        requesting_user.check_role(&UserRole::Admin)?;
        let plugin = self.get_plugin(plugin_id.clone(), requesting_user).await?;

        let infos = self.plugin_manager.load_wasm_plugin(&plugin.path).await?;
        let update: PluginForUpdate = infos.into();
        let plugin = self.update_plugin(&plugin_id, update, requesting_user).await?;
		Ok(plugin)
	}

    pub async fn update_plugin(&self, plugin_id: &str, update: PluginForUpdate, requesting_user: &ConnectedUser) -> Result<Plugin> {
        requesting_user.check_role(&UserRole::Admin)?;
		self.store.update_plugin(plugin_id, update).await?;
        let plugin = self.store.get_plugin(plugin_id).await?.ok_or(SourcesError::UnableToFindPlugin(plugin_id.to_string(), "update_plugin".to_string()))?;

        Ok(plugin)
	}

    pub async fn install_plugin(&self, plugin: PluginForInstall, requesting_user: &ConnectedUser) -> RsResult<Plugin> {
        requesting_user.check_role(&UserRole::Admin)?;
        let plugins = self.plugin_manager.plugins.read().await;
        let plugin = plugins.iter().find(|p| p.filename == plugin.path).ok_or(SourcesError::UnableToFindPlugin(plugin.path.to_string(), "install_plugin".to_string()))?;
  
        let plugin_for_add:PluginForAdd  = plugin.into();
        
        let plugin = PluginForInsert {
            id: nanoid!(),
            plugin: plugin_for_add
        };
		self.store.add_plugin(plugin.clone()).await?;
        let plugin = self.get_plugin(plugin.id, &requesting_user).await?;
		Ok(plugin)
	}

    pub async fn add_plugin(&self, plugin: PluginForAdd, requesting_user: &ConnectedUser) -> RsResult<Plugin> {
        requesting_user.check_role(&UserRole::Admin)?;
        let plugin = PluginForInsert {
            id: nanoid!(),
            plugin
        };
		self.store.add_plugin(plugin.clone()).await?;
        let plugin = self.get_plugin(plugin.id, &requesting_user).await?;
		Ok(plugin)
	}


    pub async fn remove_plugin(&self, plugin_id: &str, requesting_user: &ConnectedUser) -> RsResult<Plugin> {
        requesting_user.check_role(&UserRole::Admin)?;
        let plugin = self.store.get_plugin(&plugin_id).await?.ok_or(SourcesError::UnableToFindPlugin(plugin_id.to_string(), "get_plugin".to_string()))?;

        self.store.remove_plugin(plugin_id.to_string()).await?;
        Ok(plugin)
	}


    pub async fn exec_parse(&self, library_id: Option<String>, url: String, requesting_user: &ConnectedUser) -> RsResult<RsLink> {
		if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::UrlParser), ..Default::default() }).await?;

        Ok(self.plugin_manager.parse(url.clone(), plugins).await.unwrap_or(RsLink { platform: "link".to_owned(), kind: Some(RsLinkType::Other), id: url, ..Default::default() }))
	}

    pub async fn exec_expand(&self, library_id: Option<String>, link: RsLink, requesting_user: &ConnectedUser) -> RsResult<String> {
		if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::UrlParser), ..Default::default() }).await?;

        Ok(self.plugin_manager.expand(link.clone(), plugins).await.ok_or(Error::NotFound(format!("Unable to expand link {:?}", link)))?)
	}



    pub async fn exec_request(&self, request: RsRequest, library_id: Option<String>, savable: bool, progress: Option<Sender<RsProgress>>, requesting_user: &ConnectedUser) -> RsResult<SourceRead> {
       
        if let Some(library_id) = library_id {
            requesting_user.check_request_role(&library_id, &request)?;

        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::Request), ..Default::default() }).await?.collect();
        self.plugin_manager.request(request, savable, plugins, progress).await
        
    }

    pub async fn parse_request(&self, request: RsRequest, progress: RsProgressCallback) -> RsResult<SourceRead> {
        let ctx = YydlContext::new().await?;
        let result = ctx.request(&request, progress).await?;

        return Ok(result);
    }

    pub async fn exec_permanent(&self, request: RsRequest, library_id: Option<String>, progress: Option<Sender<RsProgress>>, requesting_user: &ConnectedUser) -> RsResult<RsRequest> {
       
        if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;

        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::Request), ..Default::default() }).await?;
        self.plugin_manager.request_permanent(request, plugins, progress).await?.ok_or(crate::Error::NotFound("Unable to get permanent link".to_string()))
        
    }

    pub async fn exec_lookup(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser) -> RsResult<Vec<RsRequest>> {
        if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::Lookup), ..Default::default() }).await?;

        self.plugin_manager.lookup(query, plugins).await
        
    }



    pub async fn exec_token_exchange(&self, plugin_id: &str, request: HashMap<String, String>, requesting_user: &ConnectedUser) -> RsResult<PluginCredential> {

        requesting_user.check_role(&UserRole::Admin)?;
        

        let plugin = self.store.get_plugin(plugin_id).await?.ok_or(SourcesError::UnableToFindPlugin(plugin_id.to_string(), "get_plugin".to_string()))?;
        
        self.plugin_manager.exchange_token(plugin, request).await
    }



    pub async fn upload_plugin(&self, reader: AsyncReadPinBox, requesting_user: &ConnectedUser) -> RsResult<()> {

        requesting_user.check_role(&UserRole::Admin)?;

        
        let mut path = get_plugin_fodler().await?;

        let name = format!("plugin_{}.wasm", nanoid!());
        path.push(name);        

        let mut file = BufWriter::new(File::create(&path).await?);
        
		tokio::pin!(reader);
		tokio::pin!(file);
		copy(&mut reader, &mut file).await?;


        Ok(())

	}
}
