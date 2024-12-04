use std::{collections::HashMap, time::Duration};

use async_recursion::async_recursion;
use extism::convert::Json;
use futures::future::ok;
use http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use rs_plugin_common_interfaces::{lookup::{RsLookupQuery, RsLookupSourceResult, RsLookupWrapper}, request::{RsRequest, RsRequestPluginRequest, RsRequestStatus}, url::RsLink, CredentialType, PluginCredential, PluginType, RsPluginRequest};

use crate::{domain::{plugin::{Plugin, PluginWithCredential}, progress::RsProgressCallback}, error::RsResult, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::{array_tools::AddOrSetArray, file_tools::{filename_from_path, get_mime_from_filename}, http_tools::{extract_header, guess_filename, parse_content_disposition}, log::log_error, video_tools::ytdl::YydlContext}, Error};

use super::{sources::{RsRequestHeader, SourceRead}, PluginManager};

fn check_if_oauth(credential: &Option<CredentialType>) -> bool {
    match credential {
        Some(CredentialType::Oauth { .. }) => true,
        _ => false,
    }
}

impl PluginManager {

    

    pub async fn exchange_token(&self, plugin: Plugin, request: HashMap<String, String>) -> RsResult<PluginCredential> {
        if let Some(pluginwasm) = self.plugins.read().await.iter().find(|p| p.filename == plugin.path) {
            let mut plugin_m = pluginwasm.plugin.lock().unwrap();
            if check_if_oauth(&pluginwasm.infos.credential_kind) {
                let plugin_settings = serde_json::to_value(&plugin.settings)?;
                let call_object: RsPluginRequest<HashMap<String, String>> = RsPluginRequest {
                    request,
                    plugin_settings,
                    ..Default::default()
                };
                let res = plugin_m.call_get_error_code::<Json<RsPluginRequest<HashMap<String, String>>>, Json<PluginCredential>>("exchange_token", Json(call_object));
                
                match res {
                    Ok(Json(res)) =>  Ok(res),
                    Err((error, code)) => Err(Error::PluginError(code, error.to_string())),
                }
               
            } else {
                
                Err(Error::PluginUnsupportedCredentialType(CredentialType::Oauth { url: "".to_string() }, plugin.credential_type))
            }
        } else {
            Err(Error::PluginNotFound(plugin.path))
        }
        
        
    }
}