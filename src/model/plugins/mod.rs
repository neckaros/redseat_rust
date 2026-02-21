


use std::collections::HashMap;

use nanoid::nanoid;
use rs_plugin_common_interfaces::{lookup::{RsLookupMetadataResultWrapper, RsLookupQuery, RsLookupSourceResult}, request::{RsGroupDownload, RsProcessingStatus, RsRequest}, url::{RsLink, RsLinkType}, ExternalImage, PluginCredential, PluginInformation, PluginType, RsPluginRequest};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{fs::{self, File}, io::{copy, BufWriter}, sync::mpsc::Sender};


use crate::{domain::{backup::Backup, library::LibraryRole, plugin::{Plugin, PluginForAdd, PluginForInsert, PluginForInstall, PluginForUpdate, PluginWasm, PluginWithCredential}, progress::{RsProgress, RsProgressCallback}, request_processing::{RequestProcessingMessage, RequestProcessingWithAction, RsRequestProcessing, RsRequestProcessingForInsert, RsRequestProcessingForUpdate}, ElementAction}, error::{RsError, RsResult}, plugins::{get_plugin_fodler, sources::{error::SourcesError, AsyncReadPinBox, SourceRead}, url::{self, PluginTarget}}, tools::{file_tools::extract_zip, http_tools::download_latest_wasm, video_tools::ytdl::YydlContext}};

use super::{error::{Error, Result}, users::{ConnectedUser, UserRole}, ModelController};

pub mod video_convert_plugin;

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
                existing.params = plugin.infos.settings.clone();
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

    pub async fn get_plugins_with_credential(&self, query: PluginQuery) -> Result<impl Iterator<Item = PluginWithCredential>> {
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
    
    pub async fn get_plugin_with_credential(&self, id: &str) -> Result<PluginWithCredential> {
		let plugin = self.store.get_plugin(id).await?.ok_or(SourcesError::UnableToFindPlugin(id.to_string(), "get_plugin_with_credential".to_string()))?;
		let credentials = self.store.get_credentials().await?;
      
        let credential = credentials.iter().find(|c| Some(&c.id) == plugin.credential.as_ref()).cloned();
        Ok(PluginWithCredential { plugin: plugin, credential })
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
        self.reload_plugins(&requesting_user).await?;
        Ok(plugin)
	}

    pub async fn remove_plugin_wasm(&self, plugin_id: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
        requesting_user.check_role(&UserRole::Admin)?;
        self.remove_plugin(plugin_id, requesting_user).await;
        let mut path = get_plugin_fodler().await?;
        path.push(plugin_id);   
        fs::remove_file(path).await?;
        self.reload_plugins(&requesting_user).await?;
        Ok(())
	}


    pub async fn exec_parse(&self, library_id: Option<String>, url: String, requesting_user: &ConnectedUser) -> RsResult<RsLink> {
		if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::UrlParser), library: library_id, ..Default::default() }).await?;

        Ok(self.plugin_manager.parse(url.clone(), plugins).await.unwrap_or(RsLink { platform: "link".to_owned(), kind: Some(RsLinkType::Other), id: url, ..Default::default() }))
	}

    pub async fn exec_expand(&self, library_id: Option<String>, link: RsLink, requesting_user: &ConnectedUser) -> RsResult<String> {
		if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::UrlParser), library: library_id, ..Default::default() }).await?;

        Ok(self.plugin_manager.expand(link.clone(), plugins).await.ok_or(Error::NotFound(format!("Unable to expand link {:?}", link)))?)
	}



    pub async fn exec_request(&self, request: RsRequest, library_id: Option<String>, savable: bool, progress: Option<Sender<RsProgress>>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<SourceRead> {

        if let Some(library_id) = &library_id {
            requesting_user.check_request_role(library_id, &request)?;

        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::Request), library: library_id, ..Default::default() }).await?.collect();
        self.plugin_manager.request(request, savable, plugins, progress, target).await

    }

    pub async fn parse_request(&self, request: RsRequest, progress: RsProgressCallback) -> RsResult<SourceRead> {
        let ctx = YydlContext::new().await?;
        let result = ctx.request(&request, progress).await?;

        return Ok(result);
    }

    pub async fn exec_permanent(&self, request: RsRequest, library_id: Option<String>, progress: Option<Sender<RsProgress>>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<RsRequest> {

        if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;

        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::Request), library: library_id, ..Default::default() }).await?.collect();
        self.plugin_manager.request_permanent(request, plugins, progress, target).await?.ok_or(crate::Error::NotFound("Unable to get permanent link".to_string()))

    }

    pub async fn exec_lookup(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<Vec<RsGroupDownload>> {
        if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::Lookup), library: library_id, ..Default::default() }).await?.collect();

        self.plugin_manager.lookup(query, plugins, target).await

    }



    pub async fn exec_lookup_metadata(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<Vec<RsLookupMetadataResultWrapper>> {
        if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::LookupMetadata), library: library_id, ..Default::default() }).await?.collect();

        self.plugin_manager.lookup_metadata(query, plugins, target).await
    }

    pub async fn exec_lookup_images(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<Vec<ExternalImage>> {
        if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::LookupMetadata), library: library_id, ..Default::default() }).await?.collect();

        self.plugin_manager.lookup_images(query, plugins, target).await
    }

    pub async fn exec_lookup_metadata_grouped(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<HashMap<String, Vec<RsLookupMetadataResultWrapper>>> {
        if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::LookupMetadata), library: library_id, ..Default::default() }).await?.collect();

        self.plugin_manager.lookup_metadata_grouped(query, plugins, target).await
    }

    pub async fn exec_lookup_images_grouped(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<HashMap<String, Vec<ExternalImage>>> {
        if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::LookupMetadata), library: library_id, ..Default::default() }).await?.collect();

        self.plugin_manager.lookup_images_grouped(query, plugins, target).await
    }

    pub async fn exec_lookup_metadata_stream(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<tokio::sync::mpsc::Receiver<Vec<RsLookupMetadataResultWrapper>>> {
        if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::LookupMetadata), library: library_id, ..Default::default() }).await?.collect();

        self.plugin_manager.lookup_metadata_stream(query, plugins, target).await
    }

    pub async fn exec_lookup_images_stream(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<tokio::sync::mpsc::Receiver<Vec<ExternalImage>>> {
        if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::LookupMetadata), library: library_id, ..Default::default() }).await?.collect();

        self.plugin_manager.lookup_images_stream(query, plugins, target).await
    }

    pub async fn exec_lookup_metadata_stream_grouped(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<tokio::sync::mpsc::Receiver<(String, Vec<RsLookupMetadataResultWrapper>)>> {
        if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::LookupMetadata), library: library_id, ..Default::default() }).await?.collect();

        self.plugin_manager.lookup_metadata_stream_grouped(query, plugins, target).await
    }

    pub async fn exec_lookup_images_stream_grouped(&self, query: RsLookupQuery, library_id: Option<String>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<tokio::sync::mpsc::Receiver<(String, Vec<ExternalImage>)>> {
        if let Some(library_id) = &library_id {
            requesting_user.check_library_role(library_id, crate::domain::library::LibraryRole::Read)?;
        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::LookupMetadata), library: library_id, ..Default::default() }).await?.collect();

        self.plugin_manager.lookup_images_stream_grouped(query, plugins, target).await
    }

    pub async fn exec_token_exchange(&self, plugin_id: &str, request: HashMap<String, String>, requesting_user: &ConnectedUser) -> RsResult<PluginCredential> {

        requesting_user.check_role(&UserRole::Admin)?;
        

        let plugin = self.store.get_plugin(plugin_id).await?.ok_or(SourcesError::UnableToFindPlugin(plugin_id.to_string(), "get_plugin".to_string()))?;
        
        self.plugin_manager.exchange_token(plugin, request).await
    }


    pub async fn refresh_repo_plugin(&self, plugin_id: &str, requesting_user: &ConnectedUser) -> RsResult<Plugin> {

        requesting_user.check_role(&UserRole::Admin)?;

        let plugin = self.store.get_plugin(plugin_id).await?.ok_or(SourcesError::UnableToFindPlugin(plugin_id.to_string(), "get_plugin".to_string()))?;
        
        let url = plugin.repo.ok_or(RsError::Error("Plugin does not have a repo".to_string()))?;
        let name = plugin.path.clone();

        let mut path = get_plugin_fodler().await?;

        download_latest_wasm(&url, path.to_str().ok_or(RsError::Error("Unable to get plugin folder path".to_string()))?, Some(&name)).await?;

        self.reload_plugin(plugin_id.to_string(), requesting_user).await

	}

    pub async fn upload_repo_plugin(&self, url: &str, requesting_user: &ConnectedUser) -> RsResult<String> {

        requesting_user.check_role(&UserRole::Admin)?;

        
        let mut path = get_plugin_fodler().await?;

        let name = format!("plugin_{}.wasm", nanoid!());    

        download_latest_wasm(url, path.to_str().ok_or(RsError::Error("Unable to get plugin folder path".to_string()))?, Some(&name)).await?;

        self.reload_plugins(&requesting_user).await?;
        path.push(name);
        Ok(path.to_string_lossy().to_string())

	}


    pub async fn upload_plugin(&self, reader: &mut (dyn tokio::io::AsyncRead + Unpin + Send), filename: &str, requesting_user: &ConnectedUser) -> RsResult<()> {

        requesting_user.check_role(&UserRole::Admin)?;


        let mut path = get_plugin_fodler().await?;
        if filename.ends_with(".wasm") {
            let name = format!("plugin_{}.wasm", nanoid!());
            path.push(name);

            let mut file = BufWriter::new(File::create(&path).await?);
            tokio::io::copy(reader, &mut file).await?;
        } else if filename.ends_with(".zip") {
            path.push(nanoid!());
            tokio::fs::create_dir_all(&path).await?;

            // Extract using reusable function
            extract_zip(reader, &path).await?;
        }


        Ok(())

	}

    // ============== Request Processing Methods ==============

    /// Check if request can be played instantly without adding to service
    pub async fn exec_check_instant(&self, request: RsRequest, library_id: &str, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<bool> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let plugins = self.get_plugins_with_credential(PluginQuery {
            kind: Some(PluginType::Request), library: Some(library_id.to_string()),
            ..Default::default()
        }).await?.collect();

        let result = self.plugin_manager.check_instant(request.clone(), plugins, target).await?;
        Ok(result.unwrap_or(request.instant.unwrap_or(false)))
    }

    /// Add a request for processing (for RequireAdd status)
    pub async fn exec_request_add(&self, request: RsRequest, library_id: &str, media_ref: Option<String>, requesting_user: &ConnectedUser, target: Option<PluginTarget>) -> RsResult<RsRequestProcessing> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;

        let plugins: Vec<PluginWithCredential> = self.get_plugins_with_credential(PluginQuery {
            kind: Some(PluginType::Request), library: Some(library_id.to_string()),
            ..Default::default()
        }).await?.collect();

        let result = self.plugin_manager.request_add(request.clone(), plugins, target).await?
            .ok_or(Error::NotFound("No plugin handled request_add".to_string()))?;

        let (plugin_id, add_response) = result;
        let id = nanoid!();

        let insert = RsRequestProcessingForInsert {
            id: id.clone(),
            processing_id: add_response.processing_id,
            plugin_id,
            eta: add_response.eta,
            media_ref,
            original_request: Some(request),
        };

        let library_store = self.store.get_library_store(library_id)?;
        library_store.add_request_processing(insert).await?;

        let processing = library_store.get_request_processing(&id).await?
            .ok_or(Error::NotFound(format!("Request processing {} not found after insert", id)))?;

        // Broadcast SSE event for new processing
        self.send_request_processing(RequestProcessingMessage {
            library: library_id.to_string(),
            processings: vec![RequestProcessingWithAction {
                action: ElementAction::Added,
                processing: processing.clone(),
            }],
        });

        Ok(processing)
    }

    /// List all active request processings for a library
    pub async fn list_request_processings(&self, library_id: &str, requesting_user: &ConnectedUser) -> RsResult<Vec<RsRequestProcessing>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;

        let library_store = self.store.get_library_store(library_id)?;
        let processings = library_store.get_all_active_request_processings().await?;
        Ok(processings)
    }

    /// Get a specific request processing
    pub async fn get_request_processing(&self, library_id: &str, processing_id: &str, requesting_user: &ConnectedUser) -> RsResult<RsRequestProcessing> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;

        let library_store = self.store.get_library_store(library_id)?;
        library_store.get_request_processing(processing_id).await?
            .ok_or(Error::NotFound(format!("Processing {} not found", processing_id)).into())
    }

    /// Get progress of a processing task (returns cached DB value)
    /// Progress is updated by the scheduled RequestProgressTask every 30 seconds
    pub async fn get_processing_progress(&self, library_id: &str, processing_nanoid: &str, requesting_user: &ConnectedUser) -> RsResult<RsRequestProcessing> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;

        let library_store = self.store.get_library_store(library_id)?;
        library_store.get_request_processing(processing_nanoid).await?
            .ok_or(Error::NotFound(format!("Processing {} not found", processing_nanoid)).into())
    }

    /// Pause a processing task
    pub async fn pause_processing(&self, library_id: &str, processing_nanoid: &str, requesting_user: &ConnectedUser) -> RsResult<RsRequestProcessing> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;

        let library_store = self.store.get_library_store(library_id)?;
        let processing = library_store.get_request_processing(processing_nanoid).await?
            .ok_or(Error::NotFound(format!("Processing {} not found", processing_nanoid)))?;

        let plugin_with_cred = self.get_plugin_with_credential(&processing.plugin_id).await?;

        self.plugin_manager.pause_processing(
            &processing.processing_id,
            &plugin_with_cred,
        ).await?;

        let update = RsRequestProcessingForUpdate {
            progress: None,
            status: Some(RsProcessingStatus::Paused),
            error: None,
            eta: None,
        };
        library_store.update_request_processing(processing_nanoid, update).await?;

        library_store.get_request_processing(processing_nanoid).await?
            .ok_or(Error::NotFound(format!("Processing {} not found", processing_nanoid)).into())
    }

    /// Remove/cancel a processing task
    pub async fn remove_processing(&self, library_id: &str, processing_nanoid: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;

        let library_store = self.store.get_library_store(library_id)?;
        let processing = library_store.get_request_processing(processing_nanoid).await?
            .ok_or(Error::NotFound(format!("Processing {} not found", processing_nanoid)))?;

        let plugin_with_cred = self.get_plugin_with_credential(&processing.plugin_id).await?;

        self.plugin_manager.remove_processing(
            &processing.processing_id,
            &plugin_with_cred,
        ).await?;

        library_store.remove_request_processing(processing_nanoid).await?;
        Ok(())
    }

    /// Process all active request processings for a library
    /// Polls plugin for progress, updates DB, and triggers download when finished
    /// Returns the number of processings that were checked
    pub async fn process_active_requests(&self, library_id: &str) -> RsResult<usize> {
        let library_store = self.store.get_library_store(library_id)?;
        let processings = library_store.get_all_active_request_processings().await?;

        let count = processings.len();

        for processing in processings {
            // Get the plugin with credentials
            let plugin_with_cred = match self.get_plugin_with_credential(&processing.plugin_id).await {
                Ok(p) => p,
                Err(e) => {
                    crate::tools::log::log_error(
                        crate::tools::log::LogServiceType::Scheduler,
                        format!("Failed to get plugin {} for processing {}: {:#}", processing.plugin_id, processing.id, e),
                    );
                    // Mark as error if plugin not found
                    let update = RsRequestProcessingForUpdate {
                        progress: None,
                        status: Some(RsProcessingStatus::Error),
                        error: Some(format!("Plugin not found: {}", e)),
                        eta: None,
                    };
                    let _ = library_store.update_request_processing(&processing.id, update).await;
                    continue;
                }
            };

            // Poll plugin for progress
            let progress = match self.plugin_manager.get_processing_progress(
                &processing.processing_id,
                &plugin_with_cred,
            ).await {
                Ok(p) => p,
                Err(e) => {
                    crate::tools::log::log_error(
                        crate::tools::log::LogServiceType::Scheduler,
                        format!("Failed to get progress for processing {}: {:#}", processing.id, e),
                    );
                    // Skip this one - may be a temporary communication error
                    continue;
                }
            };

            // Update DB with new progress
            let update = RsRequestProcessingForUpdate {
                progress: Some(progress.progress),
                status: Some(progress.status.clone()),
                error: progress.error.clone(),
                eta: progress.eta,
            };

            // Check if there was a meaningful change (status or progress)
            let has_change = progress.status != processing.status
                || progress.progress != processing.progress;

            if let Err(e) = library_store.update_request_processing(&processing.id, update).await {
                crate::tools::log::log_error(
                    crate::tools::log::LogServiceType::Scheduler,
                    format!("Failed to update processing {} in DB: {:#}", processing.id, e),
                );
                continue;
            }

            // Emit SSE event if there was a change
            if has_change {
                if let Ok(Some(updated)) = library_store.get_request_processing(&processing.id).await {
                    self.send_request_processing(RequestProcessingMessage {
                        library: library_id.to_string(),
                        processings: vec![RequestProcessingWithAction {
                            action: ElementAction::Updated,
                            processing: updated,
                        }],
                    });
                }
            }

            // If finished, trigger download
            if progress.status == RsProcessingStatus::Finished {
                // Get the final request - either from progress response or original
                let final_request = if let Some(req) = progress.request {
                    *req
                } else if let Some(req) = processing.original_request.clone() {
                    req
                } else {
                    crate::tools::log::log_error(
                        crate::tools::log::LogServiceType::Scheduler,
                        format!("No request available for finished processing {}", processing.id),
                    );
                    continue;
                };

                // Build group download with single request
                let group_download = RsGroupDownload {
                    group: false,
                    group_thumbnail_url: None,
                    group_filename: None,
                    group_mime: None,
                    requests: vec![final_request],
                    ..Default::default()
                };

                // Trigger download
                let connected_user = super::users::ConnectedUser::ServerAdmin;
                match self.download_library_url(library_id, group_download, &connected_user).await {
                    Ok(medias) => {
                        crate::tools::log::log_info(
                            crate::tools::log::LogServiceType::Scheduler,
                            format!(
                                "Successfully downloaded {} media(s) for processing {}",
                                medias.len(),
                                processing.id
                            ),
                        );
                        // Remove the processing record on success
                        if let Err(e) = library_store.remove_request_processing(&processing.id).await {
                            crate::tools::log::log_error(
                                crate::tools::log::LogServiceType::Scheduler,
                                format!("Failed to remove completed processing {}: {:#}", processing.id, e),
                            );
                        } else {
                            // Emit SSE event for deleted processing
                            self.send_request_processing(RequestProcessingMessage {
                                library: library_id.to_string(),
                                processings: vec![RequestProcessingWithAction {
                                    action: ElementAction::Deleted,
                                    processing: processing.clone(),
                                }],
                            });
                        }
                    }
                    Err(e) => {
                        crate::tools::log::log_error(
                            crate::tools::log::LogServiceType::Scheduler,
                            format!("Failed to download media for processing {}: {:#}", processing.id, e),
                        );
                        // Mark as error
                        let update = RsRequestProcessingForUpdate {
                            progress: None,
                            status: Some(RsProcessingStatus::Error),
                            error: Some(format!("Download failed: {}", e)),
                            eta: None,
                        };
                        let _ = library_store.update_request_processing(&processing.id, update).await;
                    }
                }
            } else if progress.status == RsProcessingStatus::Error {
                crate::tools::log::log_error(
                    crate::tools::log::LogServiceType::Scheduler,
                    format!(
                        "Processing {} failed with error: {:?}",
                        processing.id,
                        progress.error
                    ),
                );
            }
        }

        Ok(count)
    }
}
