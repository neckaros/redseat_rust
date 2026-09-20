use std::{collections::HashMap, time::Duration};

use async_recursion::async_recursion;
use extism::convert::Json;
use futures::future::ok;
use http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use rs_plugin_common_interfaces::{
    lookup::{RsLookupQuery, RsLookupSourceResult, RsLookupWrapper},
    provider::{RsProviderAddRequest, RsProviderAddResponse, RsProviderEntry, RsProviderPath},
    request::{RsRequest, RsRequestPluginRequest, RsRequestStatus},
    url::RsLink,
    PluginCredential, PluginType, RsPluginRequest,
};
use serde_json::json;

use crate::{
    domain::{plugin::PluginWithCredential, progress::RsProgressCallback},
    error::RsResult,
    plugins::sources::{error::SourcesError, AsyncReadPinBox, FileStreamResult},
    tools::{
        array_tools::AddOrSetArray,
        file_tools::{filename_from_path, get_mime_from_filename},
        http_tools::{extract_header, guess_filename, parse_content_disposition},
        log::log_error,
        video_tools::ytdl::YydlContext,
    },
    Error,
};

use super::{
    plugin_call_error,
    sources::{RsRequestHeader, SourceRead},
    PluginManager,
};

impl PluginManager {
    pub async fn provider_get_file(
        &self,
        path: RsProviderPath,
        plugin_with_creds: &PluginWithCredential,
    ) -> RsResult<RsRequest> {
        if let Some(plugin) = self
            .plugin_by_filename(&plugin_with_creds.plugin.path)
            .await
        {
            if plugin.infos.capabilities.contains(&PluginType::Provider) {
                let call_object: RsPluginRequest<RsProviderPath> = RsPluginRequest {
                    request: path.clone(),
                    plugin_settings: plugin_with_creds
                        .credential
                        .as_ref()
                        .map(|c| c.settings.clone())
                        .unwrap_or(json!({})),
                    credential: plugin_with_creds.credential.clone().map(|c| c.into()),
                };
                let res = plugin
                    .call_get_error_code::<Json<RsPluginRequest<RsProviderPath>>, Json<RsRequest>>(
                        "download_request",
                        Json(call_object),
                    )
                    .await;
                match res {
                    Ok(Json(res)) => Ok(res),
                    Err((_, code)) if code == 404 => {
                        Err(Error::Error(format!("Provider plugin error: {}", code)))
                    }
                    Err(error) => Err(plugin_call_error(
                        &plugin.infos.name,
                        "download_request",
                        error,
                    )),
                }
            } else {
                Err(Error::ModelNotFound(format!(
                    "provider plugin {}",
                    plugin.filename
                )))
            }
        } else {
            Err(Error::ModelNotFound(format!(
                "Unable to find plugin provider plugin {}: path:{}; list: {:?}",
                plugin_with_creds.plugin.name,
                plugin_with_creds.plugin.path,
                self.plugins.read().await
            )))
        }
    }

    pub async fn provider_upload_file_request(
        &self,
        path: RsProviderAddRequest,
        plugin_with_creds: &PluginWithCredential,
    ) -> RsResult<RsProviderAddResponse> {
        if let Some(plugin) = self
            .plugin_by_filename(&plugin_with_creds.plugin.path)
            .await
        {
            if plugin.infos.capabilities.contains(&PluginType::Provider) {
                let call_object: RsPluginRequest<RsProviderAddRequest> = RsPluginRequest {
                    request: path,
                    plugin_settings: serde_json::to_value(
                        plugin_with_creds.plugin.settings.clone(),
                    )?,
                    credential: plugin_with_creds.credential.clone().map(|c| c.into()),
                };
                let res = plugin.call_get_error_code::<Json<RsPluginRequest<RsProviderAddRequest>>, Json<RsProviderAddResponse>>("upload_request", Json(call_object)).await;
                match res {
                    Ok(Json(res)) => Ok(res),
                    Err((_, code)) if code == 404 => {
                        Err(Error::Error(format!("Provider plugin error: {}", code)))
                    }
                    Err(error) => Err(plugin_call_error(
                        &plugin.infos.name,
                        "upload_request",
                        error,
                    )),
                }
            } else {
                Err(Error::ModelNotFound(format!(
                    "provider plugin {}",
                    plugin.filename
                )))
            }
        } else {
            Err(Error::ModelNotFound(format!(
                "provider plugin {}",
                plugin_with_creds.plugin.name
            )))
        }
    }

    pub async fn provider_upload_parse_response(
        &self,
        response: String,
        plugin_with_creds: &PluginWithCredential,
    ) -> RsResult<RsProviderEntry> {
        if let Some(plugin) = self
            .plugin_by_filename(&plugin_with_creds.plugin.path)
            .await
        {
            if plugin.infos.capabilities.contains(&PluginType::Provider) {
                let call_object: RsPluginRequest<String> = RsPluginRequest {
                    request: response,
                    plugin_settings: serde_json::to_value(
                        plugin_with_creds.plugin.settings.clone(),
                    )?,
                    credential: plugin_with_creds.credential.clone().map(|c| c.into()),
                };
                let res = plugin
                    .call_get_error_code::<Json<RsPluginRequest<String>>, Json<RsProviderEntry>>(
                        "upload_response",
                        Json(call_object),
                    )
                    .await;
                match res {
                    Ok(Json(res)) => Ok(res),
                    Err((_, code)) if code == 404 => {
                        Err(Error::Error(format!("Provider plugin error: {}", code)))
                    }
                    Err(error) => Err(plugin_call_error(
                        &plugin.infos.name,
                        "upload_response",
                        error,
                    )),
                }
            } else {
                Err(Error::ModelNotFound(format!(
                    "provider plugin {}",
                    plugin.filename
                )))
            }
        } else {
            Err(Error::ModelNotFound(format!(
                "provider plugin {}",
                plugin_with_creds.plugin.name
            )))
        }
    }

    pub async fn provider_remove_file(
        &self,
        path: RsProviderPath,
        plugin_with_creds: &PluginWithCredential,
    ) -> RsResult<()> {
        let source = path.source.clone();
        if let Some(plugin) = self
            .plugin_by_filename(&plugin_with_creds.plugin.path)
            .await
        {
            if plugin.infos.capabilities.contains(&PluginType::Provider) {
                let call_object: RsPluginRequest<RsProviderPath> = RsPluginRequest {
                    request: path,
                    plugin_settings: plugin_with_creds
                        .credential
                        .as_ref()
                        .map(|c| c.settings.clone())
                        .unwrap_or(json!({})),
                    credential: plugin_with_creds.credential.clone().map(|c| c.into()),
                };
                let res = plugin
                    .call_get_error_code::<Json<RsPluginRequest<RsProviderPath>>, ()>(
                        "remove_file",
                        Json(call_object),
                    )
                    .await;
                match res {
                    Ok(()) => Ok(()),
                    Err((_, code)) if code == 404 => {
                        Err(SourcesError::NotFound(Some(source)).into())
                    }
                    Err(error) => Err(plugin_call_error(&plugin.infos.name, "remove_file", error)),
                }
            } else {
                Err(Error::ModelNotFound(format!(
                    "provider plugin {}",
                    plugin.filename
                )))
            }
        } else {
            Err(Error::ModelNotFound(format!(
                "provider plugin {}",
                plugin_with_creds.plugin.name
            )))
        }
    }

    pub async fn provider_info_file(
        &self,
        path: RsProviderPath,
        plugin_with_creds: &PluginWithCredential,
    ) -> RsResult<RsProviderEntry> {
        if let Some(plugin) = self
            .plugin_by_filename(&plugin_with_creds.plugin.path)
            .await
        {
            if plugin.infos.capabilities.contains(&PluginType::Provider) {
                let call_object: RsPluginRequest<RsProviderPath> = RsPluginRequest {
                    request: path,
                    plugin_settings: plugin_with_creds
                        .credential
                        .as_ref()
                        .map(|c| c.settings.clone())
                        .unwrap_or(json!({})),
                    credential: plugin_with_creds.credential.clone().map(|c| c.into()),
                };
                let res = plugin.call_get_error_code::<Json<RsPluginRequest<RsProviderPath>>, Json<RsProviderEntry>>("file_info", Json(call_object)).await;
                match res {
                    Ok(Json(p)) => Ok(p),
                    Err((_, code)) if code == 404 => {
                        Err(Error::Error(format!("Provider plugin error: {}", code)))
                    }
                    Err(error) => Err(plugin_call_error(&plugin.infos.name, "file_info", error)),
                }
            } else {
                Err(Error::ModelNotFound(format!(
                    "provider plugin {}",
                    plugin.filename
                )))
            }
        } else {
            Err(Error::ModelNotFound(format!(
                "provider plugin {}",
                plugin_with_creds.plugin.name
            )))
        }
    }
}
