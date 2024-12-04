use std::{collections::HashMap, time::Duration};

use async_recursion::async_recursion;
use extism::convert::Json;
use futures::future::ok;
use http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use rs_plugin_common_interfaces::{lookup::{RsLookupQuery, RsLookupSourceResult, RsLookupWrapper}, provider::{RsProviderAddRequest, RsProviderAddResponse, RsProviderEntry, RsProviderPath}, request::{RsRequest, RsRequestPluginRequest, RsRequestStatus}, url::RsLink, PluginCredential, PluginType, RsPluginRequest};
use serde_json::json;

use crate::{domain::{plugin::PluginWithCredential, progress::RsProgressCallback}, error::RsResult, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::{array_tools::AddOrSetArray, file_tools::{filename_from_path, get_mime_from_filename}, http_tools::{extract_header, guess_filename, parse_content_disposition}, log::log_error, video_tools::ytdl::YydlContext}, Error};

use super::{sources::{RsRequestHeader, SourceRead}, PluginManager};


impl PluginManager {

    pub async fn provider_get_file(&self, path: RsProviderPath, plugin_with_creds: &PluginWithCredential) -> RsResult<RsRequest>{
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_creds.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            if plugin.infos.capabilities.contains(&PluginType::Provider) {
                let call_object: RsPluginRequest<RsProviderPath> = RsPluginRequest {
                    request: path,
                    plugin_settings: json!({}),
                    credential: plugin_with_creds.credential.clone().map(|c| c.into())
                };
                let res = plugin_m.call_get_error_code::<Json<RsPluginRequest<RsProviderPath>>, Json<RsRequest>>("download_request", Json(call_object));
                match res {
                    Ok(Json(res)) => Ok(res),
                    Err((error, code)) =>  {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request get gile: {} {:?}", code, error));
                            Err(Error::NotFound)
                        } else {
                            Err(Error::Error(format!("Provider plugin error: {}", code)))
                        }
                    },
                }
            } else {
                Err(Error::ModelNotFound(format!("provider plugin {}", plugin.filename)))
            }
        } else {
            Err(Error::ModelNotFound(format!("provider plugin {}", plugin_with_creds.plugin.name)))
        }

    }

    pub async fn provider_upload_file_request(&self, path: RsProviderAddRequest, plugin_with_creds: &PluginWithCredential) -> RsResult<RsProviderAddResponse>{
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_creds.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            if plugin.infos.capabilities.contains(&PluginType::Provider) {
                let call_object: RsPluginRequest<RsProviderAddRequest> = RsPluginRequest {
                    request: path,
                    plugin_settings: serde_json::to_value(plugin_with_creds.plugin.settings.clone())?,
                    credential: plugin_with_creds.credential.clone().map(|c| c.into())
                };
                let res = plugin_m.call_get_error_code::<Json<RsPluginRequest<RsProviderAddRequest>>, Json<RsProviderAddResponse>>("upload_request", Json(call_object));
                match res {
                    Ok(Json(res)) => Ok(res),
                    Err((error, code)) =>  {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request upload file: {} {:?}", code, error));
                            Err(Error::NotFound)
                        } else {
                            Err(Error::Error(format!("Provider plugin error: {}", code)))
                        }
                    },
                }
            } else {
                Err(Error::ModelNotFound(format!("provider plugin {}", plugin.filename)))
            }
        } else {
            Err(Error::ModelNotFound(format!("provider plugin {}", plugin_with_creds.plugin.name)))
        }
    }

    pub async fn provider_upload_parse_response(&self, response: String, plugin_with_creds: &PluginWithCredential) -> RsResult<RsProviderEntry>{
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_creds.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            if plugin.infos.capabilities.contains(&PluginType::Provider) {
                let call_object: RsPluginRequest<String> = RsPluginRequest {
                    request: response,
                    plugin_settings: serde_json::to_value(plugin_with_creds.plugin.settings.clone())?,
                    credential: plugin_with_creds.credential.clone().map(|c| c.into())
                };
                let res = plugin_m.call_get_error_code::<Json<RsPluginRequest<String>>, Json<RsProviderEntry>>("upload_response", Json(call_object));
                match res {
                    Ok(Json(res)) => Ok(res),
                    Err((error, code)) =>  {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request upload file: {} {:?}", code, error));
                            Err(Error::NotFound)
                        } else {
                            Err(Error::Error(format!("Provider plugin error: {}", code)))
                        }
                    },
                }
            } else {
                Err(Error::ModelNotFound(format!("provider plugin {}", plugin.filename)))
            }
        } else {
            Err(Error::ModelNotFound(format!("provider plugin {}", plugin_with_creds.plugin.name)))
        }
    }
    
    pub async fn provider_add_file(&self, add: RsProviderAddRequest, plugin: PluginWithCredential) -> RsResult<RsRequest>{
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            if plugin.infos.capabilities.contains(&PluginType::Provider) {
                let res = plugin_m.call_get_error_code::<Json<RsProviderAddRequest>, Json<RsRequest>>("add", Json(add));
                match res {
                    Ok(Json(res)) => Ok(res),
                    Err((_, code)) =>  Err(Error::from_code(code)),
                }
            } else {
                Err(Error::ModelNotFound(format!("provider plugin {}", plugin.filename)))
            }
        } else {
            Err(Error::ModelNotFound(format!("provider plugin {}", plugin.plugin.name)))
        }
      
    }
}