use std::{collections::HashMap, time::Duration};

use async_recursion::async_recursion;
use extism::convert::Json;
use futures::future::ok;
use http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use rs_plugin_common_interfaces::{lookup::{RsLookupQuery, RsLookupSourceResult, RsLookupWrapper}, provider::{RsProviderAddRequest, RsProviderPath}, request::{RsRequest, RsRequestPluginRequest, RsRequestStatus}, url::RsLink, PluginCredential, PluginType};

use crate::{domain::{plugin::PluginWithCredential, progress::RsProgressCallback}, error::RsResult, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::{array_tools::AddOrSetArray, file_tools::{filename_from_path, get_mime_from_filename}, http_tools::{extract_header, guess_filename, parse_content_disposition}, log::log_error, video_tools::ytdl::YydlContext}, Error};

use super::{sources::{RsRequestHeader, SourceRead}, PluginManager};


impl PluginManager {

    pub async fn provider_get_file(&self, path: RsProviderPath, plugin: &PluginWithCredential) -> RsResult<RsRequest>{
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            if plugin.infos.capabilities.contains(&PluginType::Provider) {
                let res = plugin_m.call_get_error_code::<Json<RsProviderPath>, Json<RsRequest>>("get", Json(path));
                match res {
                    Ok(Json(res)) => Ok(res),
                    Err((error, code)) =>  {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request {} {:?}", code, error));
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
            Err(Error::ModelNotFound(format!("provider plugin {}", plugin.plugin.name)))
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