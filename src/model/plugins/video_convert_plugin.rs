use extism::convert::Json;
use rs_plugin_common_interfaces::{video::{RsVideoCapabilities, RsVideoTranscodeCancelResponse, RsVideoTranscodeJob, RsVideoTranscodeJobPluginAction, RsVideoTranscodeJobPluginRequest, RsVideoTranscodeJobStatus, VideoConvertRequest}, PluginCredential, PluginType, RsRequest};

use crate::{domain::{plugin::{self, PluginWithCredential}, progress::RsProgress}, error::RsResult, model::{plugins::PluginQuery, users::{ConnectedUser, UserRole}, ModelController}, plugins::PluginManager, tools::log::log_error};
use tokio::sync::mpsc::Sender;

/// Capabilities for video conversion plugins
impl ModelController {
    pub async fn get_all_convert_capabilities(&self, library_id: Option<String>, requesting_user: &ConnectedUser) -> RsResult<Vec<(String, RsVideoCapabilities)>> {
        if let Some(library_id) = library_id {
            requesting_user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;

        } else {
            requesting_user.check_role(&UserRole::Admin)?;
        }
        let plugins= self.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::VideoConvert), ..Default::default() }).await?;
        self.plugin_manager.get_all_convert_capabilities(plugins).await
    }

    pub async fn get_convert_capabilities(&self, plugin_id: &str) -> RsResult<RsVideoCapabilities> {
        let plugin= self.get_plugin_with_credential(plugin_id).await?;
        self.plugin_manager.get_convert_capabilities(plugin).await
    }
}
impl PluginManager { 
    pub async fn get_all_convert_capabilities(&self, plugins: impl Iterator<Item = PluginWithCredential>) -> RsResult<Vec<(String, RsVideoCapabilities)>> {
        let mut results = vec![];
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                
                println!("PLUGIN {}", plugin_with_cred.plugin.name);
                if plugin.infos.capabilities.contains(&PluginType::VideoConvert) {
                    
                    let res = plugin_m.call_get_error_code::<&str, Json<Vec<RsVideoCapabilities>>>("get_convert_capabilities", "");
                    if let Ok(Json(res)) = res {
                        for capability in res {
                            results.push((plugin_with_cred.plugin.name.clone(), capability));
                        }
         
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request get_convert_capabilities {} {:?}", code, error))
                        }
                    }
                    
                }
            }
        }
        Ok(results)
    }

    pub async fn get_convert_capabilities(&self, plugin_with_cred: PluginWithCredential) -> RsResult<RsVideoCapabilities> {
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            
            println!("PLUGIN {}", plugin_with_cred.plugin.name);
            if plugin.infos.capabilities.contains(&PluginType::VideoConvert) {
                let res = plugin_m.call_get_error_code::<Json<PluginCredential>, Json<RsVideoCapabilities>>("get_convert_capabilities", Json(plugin_with_cred.credential.map(|t| t.into()).unwrap_or_default())).map_err(|x| crate::error::Error::PluginError(x.1, plugin_with_cred.plugin.id))?;
                return Ok(res.into_inner())  
            } else {
                return Err(crate::error::RsError::PluginUnsupportedCall(plugin_with_cred.plugin.id, "get_convert_capabilities".to_string()));
            }
        } else {
            return Err(crate::error::RsError::PluginNotFound(plugin_with_cred.plugin.id));
        }
    }
}



/// Submit conversion request
impl ModelController {
    pub async fn convert_submit(&self, request: RsRequest, job: RsVideoTranscodeJobPluginRequest, plugin_id: &str) -> RsResult<RsVideoTranscodeJobStatus> {
        
        let plugin= self.get_plugin_with_credential(plugin_id).await?;
        self.plugin_manager.convert_submit(plugin, job).await
        
    }
    pub async fn convert_submit_media(&self, library_id: &str, media_id: &str, request: VideoConvertRequest, plugin_id: &str) -> RsResult<RsVideoTranscodeJobStatus> {
        
        let plugin_with_credentials= self.get_plugin_with_credential(plugin_id).await?;
        let temp_url =ModelController::get_temporary_read_url(library_id, media_id, Some(21600)).await?;
        let jobrequest = RsVideoTranscodeJobPluginRequest {
            job: RsVideoTranscodeJob {
                source: RsRequest { url: temp_url, ..Default::default() },
                request
            },
            credentials: plugin_with_credentials.credential.clone().map(|r| r.into()).unwrap_or_default(),
        };
        self.plugin_manager.convert_submit(plugin_with_credentials, jobrequest).await
        
    }
}
impl PluginManager { 
    pub async fn convert_submit(&self, plugin_with_cred: PluginWithCredential, job: RsVideoTranscodeJobPluginRequest) -> RsResult<RsVideoTranscodeJobStatus> {
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            
            println!("PLUGIN {} - convert submit", plugin_with_cred.plugin.name);
            if plugin.infos.capabilities.contains(&PluginType::VideoConvert) {
                let res = plugin_m.call_get_error_code::<Json<RsVideoTranscodeJobPluginRequest>, Json<RsVideoTranscodeJobStatus>>("convert", Json(job)).map_err(|x| crate::error::Error::PluginError(x.1, plugin_with_cred.plugin.id))?;
                return Ok(res.into_inner())  
            } else {
                return Err(crate::error::RsError::PluginUnsupportedCall(plugin_with_cred.plugin.id, "convert".to_string()));
            }
        } else {
            return Err(crate::error::RsError::PluginNotFound(plugin_with_cred.plugin.id));
        }
    }
}

/// Submit conversion status
impl ModelController {
    pub async fn convert_status(&self,job_id: &str, plugin_id: &str) -> RsResult<RsVideoTranscodeJobStatus> {
        
        let plugin= self.get_plugin_with_credential(plugin_id).await?;
        self.plugin_manager.convert_status(plugin, job_id).await
        
    }
}
impl PluginManager { 
    pub async fn convert_status(&self, plugin_with_cred: PluginWithCredential, job_id: &str) -> RsResult<RsVideoTranscodeJobStatus> {
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            
            println!("PLUGIN {} - convert_status", plugin_with_cred.plugin.name);
            let action = RsVideoTranscodeJobPluginAction { job_id: job_id.to_string(), credentials: plugin_with_cred.credential.map(|p| p.into()).unwrap_or_default() };
            if plugin.infos.capabilities.contains(&PluginType::VideoConvert) {
                let res = plugin_m.call_get_error_code::<Json<RsVideoTranscodeJobPluginAction>, Json<RsVideoTranscodeJobStatus>>("convert_status", Json(action)).map_err(|x| crate::error::Error::PluginError(x.1, plugin_with_cred.plugin.id))?;
                return Ok(res.into_inner())  
            } else {
                return Err(crate::error::RsError::PluginUnsupportedCall(plugin_with_cred.plugin.id, "convert_status".to_string()));
            }
        } else {
            return Err(crate::error::RsError::PluginNotFound(plugin_with_cred.plugin.id));
        }
    }
}

/// Submit conversion cancel request
impl ModelController {
    pub async fn convert_cancel(&self, request: RsRequest, job_id: &str, plugin_id: &str) -> RsResult<RsVideoTranscodeCancelResponse> {
        
        let plugin= self.get_plugin_with_credential(plugin_id).await?;
        self.plugin_manager.convert_cancel(plugin, job_id).await
        
    }
}
impl PluginManager { 
    pub async fn convert_cancel(&self, plugin_with_cred: PluginWithCredential, job_id: &str) -> RsResult<RsVideoTranscodeCancelResponse> {
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            
            println!("PLUGIN {}", plugin_with_cred.plugin.name);
            if plugin.infos.capabilities.contains(&PluginType::VideoConvert) {
                let action = RsVideoTranscodeJobPluginAction { job_id: job_id.to_string(), credentials: plugin_with_cred.credential.map(|p| p.into()).unwrap_or_default() };

                let res = plugin_m.call_get_error_code::<Json<RsVideoTranscodeJobPluginAction>, Json<RsVideoTranscodeCancelResponse>>("convert_cancel", Json(action)).map_err(|x| crate::error::Error::PluginError(x.1, plugin_with_cred.plugin.id))?;
                return Ok(res.into_inner())  
            } else {
                return Err(crate::error::RsError::PluginUnsupportedCall(plugin_with_cred.plugin.id, "convert_cancel".to_string()));
            }
        } else {
            return Err(crate::error::RsError::PluginNotFound(plugin_with_cred.plugin.id));
        }
    }
}



/// get conversion link
impl ModelController {
    pub async fn convert_link(&self,job_id: &str, plugin_id: &str) -> RsResult<RsRequest> {
        
        let plugin= self.get_plugin_with_credential(plugin_id).await?;
        self.plugin_manager.convert_link(plugin, job_id).await
        
    }
}
impl PluginManager { 
    pub async fn convert_link(&self, plugin_with_cred: PluginWithCredential, job_id: &str) -> RsResult<RsRequest> {
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            
            println!("PLUGIN {} - convert_link", plugin_with_cred.plugin.name);
            if plugin.infos.capabilities.contains(&PluginType::VideoConvert) {
                let action = RsVideoTranscodeJobPluginAction { job_id: job_id.to_string(), credentials: plugin_with_cred.credential.map(|p| p.into()).unwrap_or_default() };
                let res = plugin_m.call_get_error_code::<Json<RsVideoTranscodeJobPluginAction>, Json<RsRequest>>("convert_link", Json(action)).map_err(|x| crate::error::Error::PluginError(x.1, plugin_with_cred.plugin.id))?;
                return Ok(res.into_inner())  
            } else {
                return Err(crate::error::RsError::PluginUnsupportedCall(plugin_with_cred.plugin.id, "convert_link".to_string()));
            }
        } else {
            return Err(crate::error::RsError::PluginNotFound(plugin_with_cred.plugin.id));
        }
    }
}


/// Submit conversion clean
impl ModelController {
    pub async fn convert_clean(&self,job_id: &str, plugin_id: &str) -> RsResult<RsVideoTranscodeJobStatus> {
        
        let plugin= self.get_plugin_with_credential(plugin_id).await?;
        self.plugin_manager.convert_clean(plugin, job_id).await
        
    }
}
impl PluginManager { 
    pub async fn convert_clean(&self, plugin_with_cred: PluginWithCredential, job_id: &str) -> RsResult<RsVideoTranscodeJobStatus> {
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            
            println!("PLUGIN {} - convert_clean", plugin_with_cred.plugin.name);
            let action = RsVideoTranscodeJobPluginAction { job_id: job_id.to_string(), credentials: plugin_with_cred.credential.map(|p| p.into()).unwrap_or_default() };
            if plugin.infos.capabilities.contains(&PluginType::VideoConvert) {
                let res = plugin_m.call_get_error_code::<Json<RsVideoTranscodeJobPluginAction>, Json<RsVideoTranscodeJobStatus>>("convert_clean", Json(action)).map_err(|x| crate::error::Error::PluginError(x.1, plugin_with_cred.plugin.id))?;
                return Ok(res.into_inner())  
            } else {
                return Err(crate::error::RsError::PluginUnsupportedCall(plugin_with_cred.plugin.id, "convert_clean".to_string()));
            }
        } else {
            return Err(crate::error::RsError::PluginNotFound(plugin_with_cred.plugin.id));
        }
    }
}