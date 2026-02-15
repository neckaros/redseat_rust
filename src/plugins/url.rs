use std::{collections::HashMap, time::Duration};

use async_recursion::async_recursion;
use extism::convert::Json;
use futures::future::ok;
use http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use rs_plugin_common_interfaces::{lookup::{RsLookupMetadataResultWithImages, RsLookupQuery, RsLookupSourceResult, RsLookupWrapper}, request::{RsProcessingActionRequest, RsProcessingProgress, RsRequest, RsRequestAddResponse, RsRequestPluginRequest, RsRequestStatus}, url::RsLink, PluginCredential, PluginType};

use crate::{Error, domain::{plugin::PluginWithCredential, progress::RsProgressCallback}, error::RsResult, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::{array_tools::AddOrSetArray, file_tools::{filename_from_path, get_mime_from_filename}, http_tools::{extract_header, guess_filename, parse_content_disposition}, log::{self, log_error, log_info}, video_tools::ytdl::YydlContext}};

use super::{sources::{RsRequestHeader, SourceRead}, PluginManager};

/// Optional targeting for specific plugin by ID or name
#[derive(Debug, Clone, Default)]
pub struct PluginTarget {
    pub plugin_id: Option<String>,
    pub plugin_name: Option<String>,
}

impl PluginManager {

    /// Filter plugins based on target specification
    /// Priority: target.plugin_id > target.plugin_name > request.plugin_id > request.plugin_name > all plugins
    fn filter_plugins_by_target(
        plugins: Vec<PluginWithCredential>,
        target: &Option<PluginTarget>,
        request: Option<&RsRequest>,
    ) -> crate::Result<Vec<PluginWithCredential>> {
        // Priority 1: target argument plugin_id
        if let Some(ref t) = target {
            if let Some(ref id) = t.plugin_id {
                return Self::find_plugin_by_id(plugins, id);
            }
            if let Some(ref name) = t.plugin_name {
                return Self::find_plugin_by_name(plugins, name);
            }
        }

        // Priority 2: request's plugin_id/plugin_name (fallback)
        if let Some(req) = request {
            if let Some(ref id) = req.plugin_id {
                return Self::find_plugin_by_id(plugins, id);
            }
            if let Some(ref name) = req.plugin_name {
                return Self::find_plugin_by_name(plugins, name);
            }
        }

        // No filter specified → return all plugins
        Ok(plugins)
    }

    fn find_plugin_by_id(plugins: Vec<PluginWithCredential>, id: &str) -> crate::Result<Vec<PluginWithCredential>> {
        if let Some(plugin) = plugins.into_iter().find(|p| p.plugin.id == id) {
            Ok(vec![plugin])
        } else {
            Err(Error::NotFound(format!("Plugin with id '{}' not found", id)))
        }
    }

    fn find_plugin_by_name(plugins: Vec<PluginWithCredential>, name: &str) -> crate::Result<Vec<PluginWithCredential>> {
        if let Some(plugin) = plugins.into_iter().find(|p| p.plugin.name == name) {
            Ok(vec![plugin])
        } else {
            Err(Error::NotFound(format!("Plugin with name '{}' not found", name)))
        }
    }

    pub async fn parse(&self, url: String, plugins: impl Iterator<Item = PluginWithCredential>) -> Option<RsLink>{
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                if plugin.infos.capabilities.contains(&PluginType::UrlParser) {
                    let res = plugin_m.call_get_error_code::<&str, Json<RsLink>>("parse", &url);
                    if let Ok(Json(res)) = res {
                        return Some(res)
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request parse: {} {:?}", code, error))
                        }
                    }
                    
                }
            }
        }
        None
    }

    
    pub async fn expand(&self, link: RsLink, plugins: impl Iterator<Item = PluginWithCredential>) -> Option<String>{
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                if plugin.infos.capabilities.contains(&PluginType::UrlParser) {
                    let res = plugin_m.call_get_error_code::<Json<RsLink>, &str>("expand", Json(link.clone()));
                    if let Ok(res) = res {
                        return Some(res.to_string())
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request expand {} {:?}", code, error))
                        }
                    }
                    
                }
            }
        }
        if link.platform == "link" && link.id.starts_with("http") {
            Some(link.id)
        } else {
            None
        }
    }


    pub async fn renew_crendentials(&self, plugin_with_cred: PluginWithCredential) -> crate::Result<PluginCredential>{
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            let Json(res) = plugin_m.call::<Json<Option<PluginCredential>>, Json<PluginCredential>>("renew_crendentials", Json(plugin_with_cred.credential.map(PluginCredential::from)))?;
            Ok(res)
        } else {
            Err(crate::Error::Error(format!("Plugin not found: {}", plugin_with_cred.plugin.path)))
        }

        
    }


    pub async fn fill_infos(&self, request: &mut RsRequest) {
        let ctx = YydlContext::new().await;
        if let Ok(ctx) = ctx {
            let video = ctx.request_infos(request).await;
            if let Ok(Some(video)) = video {
                if let Some(tags) = video.tags {
                    request.tags.add_or_set(tags);
                }
                if let Some(person) = video.uploader {
                    request.people.add_or_set(vec![person]);
                }
                if let Some(description) = video.description {
                    if request.description.is_none() {
                        request.description = Some(description);
                    }
                    
                }
            }
        }
    }

    #[async_recursion]
    pub async fn request(&self, mut request: RsRequest, _savable: bool, plugins: Vec<PluginWithCredential>, _progress: RsProgressCallback, target: Option<PluginTarget>) -> RsResult<SourceRead> {
        let plugins = Self::filter_plugins_by_target(plugins, &target, None)?;
        let initial_request = request.clone();
        
        let client = reqwest::Client::new();
        let r = client.head(&request.url).add_request_headers(&request, &None)?;
        let r = r.timeout(Duration::from_secs(3)).send().await;
        if let Ok(heads) = r {
            let headers = heads.headers();
            if let Some(mime) = extract_header(headers, CONTENT_TYPE) {
                request.mime = Some(mime.to_string())
            }
            if let Some(size) = extract_header(headers, CONTENT_LENGTH).and_then(|c| c.parse::<u64>().ok()) {
                request.size = Some(size);
            }
            if let Some(_filename) = extract_header(headers, CONTENT_DISPOSITION).and_then(parse_content_disposition) {
                //println!("dispo {}", filename);
            } else {
                //let filename = guess_filename(&request.url, &request.mime);
                //println!("filename {}", filename);
            }
        }

        let mut processed_request = None;
        for plugin_with_cred in plugins.iter() {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                
                if plugin.infos.capabilities.contains(&PluginType::Request) {
                    let req = RsRequestPluginRequest {
                        request: request.clone(),
                        credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                        params: plugin_with_cred.credential.as_ref()
                            .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
                    };
                    
                    //println!("call plugin request {:?}: {}", plugin.path, request.url);
                    let res = plugin_m.call_get_error_code::<Json<RsRequestPluginRequest>, Json<RsRequest>>("process", Json(req));
                    //println!("called plugin request {:?}", plugin.path);
                    if let Ok(Json(mut res)) = res {
                        log_info(crate::tools::log::LogServiceType::Plugin, format!("Request processed by plugin {}", plugin_with_cred.plugin.name));
                        res.plugin_id = Some(plugin_with_cred.plugin.id.clone());
                        res.plugin_name = Some(plugin_with_cred.plugin.name.clone());
                        if res.mime.is_none() {
                            res.mime = get_mime_from_filename(&res.url);
                        }
                        if res.filename.is_none() {
                            res.filename = filename_from_path(&res.url);
                        }
                        if res.status == RsRequestStatus::FinalPublic {
                            println!("ok request: {:?}", res);
                            return Ok(SourceRead::Request(res));
                         }
                        processed_request = Some(res);
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request ({}): {} {:?}", plugin.filename, code, error))
                        }
                    }
                    
                }
            }
        }
        if let Some(processed) = processed_request {

            if processed.status == RsRequestStatus::Intermediate && processed != initial_request {
                println!("recurse request");
                let recursed = self.request(processed, false, plugins, _progress, target).await?;
                return Ok(recursed);
            } else if processed.status == RsRequestStatus::Unprocessed {
                return Err(Error::NotFound("Unable to process request".to_string()));
            } else {
                return Ok(SourceRead::Request(processed));
            }
        }
        if request.status == RsRequestStatus::NeedParsing || request.url.contains(".m3u8") || request.url.contains(".mpd") || request.mime.as_deref().unwrap_or("no") == "application/vnd.apple.mpegurl" {
            let mut result = request.clone();
            result.status = RsRequestStatus::NeedParsing;
            Ok(SourceRead::Request(result))

        } else {
            request.status = RsRequestStatus::FinalPublic;
            Ok(SourceRead::Request(request))
        }
        
    }

    pub async fn request_permanent(&self, request: RsRequest, plugins: Vec<PluginWithCredential>, _progress: RsProgressCallback, target: Option<PluginTarget>) -> RsResult<Option<RsRequest>> {
        if request.permanent {
            Ok(Some(request))
        } else {
            let plugins = Self::filter_plugins_by_target(plugins, &target, None)?;
            for plugin_with_cred in plugins {
                if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                    let mut plugin_m = plugin.plugin.lock().unwrap();
                    if plugin.infos.capabilities.contains(&PluginType::Request) {
                        let req = RsRequestPluginRequest {
                            request: request.clone(),
                            credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                            params: plugin_with_cred.credential.as_ref()
                                .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
                        };
                        log_info(crate::tools::log::LogServiceType::Plugin, format!("call plugin request permanent  {:?}", plugin.infos.name));
                        let res = plugin_m.call_get_error_code::<Json<RsRequestPluginRequest>, Json<RsRequest>>("request_permanent", Json(req));
                        if let Ok(Json(mut res)) = res {
                            log_info(crate::tools::log::LogServiceType::Plugin, format!("got plugin request permanent  {:?}", plugin.infos.name));
                            res.plugin_id = Some(plugin_with_cred.plugin.id.clone());
                            res.plugin_name = Some(plugin_with_cred.plugin.name.clone());
                            if res.mime.is_none() {
                                res.mime = get_mime_from_filename(&res.url);
                            }
                            if res.filename.is_none() {
                                res.filename = filename_from_path(&res.url);
                            }
                            return Ok(Some(res));
                            
                            
                        } else if let Err((error, code)) = res {
                            if code != 404 {
                                log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request permanent {} {:?}", code, error))
                            }
                        }
                        
                    }
                }
            }
            Ok(None)
        }
    }

    pub async fn lookup(&self, query: RsLookupQuery, plugins: Vec<PluginWithCredential>, target: Option<PluginTarget>) -> RsResult<Vec<RsRequest>> {
        let plugins = Self::filter_plugins_by_target(plugins, &target, None)?;
        let mut results = vec![];
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();

                println!("PLUGIN {}", plugin_with_cred.plugin.name);
                if plugin.infos.capabilities.contains(&PluginType::Lookup) {
                    let wrapped_query = RsLookupWrapper {
                        query: query.clone(),
                        credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                        params: plugin_with_cred.credential.as_ref()
                            .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
                    };
                    let res = plugin_m.call_get_error_code::<Json<RsLookupWrapper>, Json<RsLookupSourceResult>>("lookup", Json(wrapped_query));
                    if let Ok(Json(res)) = res {                        if let RsLookupSourceResult::Requests(mut request) = res {
                            log_info(crate::tools::log::LogServiceType::Plugin, format!("Lookup result from plugin {}", plugin_with_cred.plugin.name));
                            for req in &mut request {
                                req.plugin_id = Some(plugin_with_cred.plugin.id.clone());
                                req.plugin_name = Some(plugin_with_cred.plugin.name.clone());
                            }
                            results.append(&mut request)
                        }
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request lookup {} {:?}", code, error))
                        }
                    }

                }
            }
        }
        Ok(results)
    }

    pub async fn lookup_metadata(&self, query: RsLookupQuery, plugins: Vec<PluginWithCredential>, target: Option<PluginTarget>) -> RsResult<Vec<RsLookupMetadataResultWithImages>> {
        let plugins = Self::filter_plugins_by_target(plugins, &target, None)?;
        let mut results = vec![];
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();

                if plugin.infos.capabilities.contains(&PluginType::LookupMetadata) {
                    let wrapped_query = RsLookupWrapper {
                        query: query.clone(),
                        credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                        params: plugin_with_cred.credential.as_ref()
                            .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
                    };
                    let res = plugin_m.call_get_error_code::<Json<RsLookupWrapper>, Json<Vec<RsLookupMetadataResultWithImages>>>("lookup_metadata", Json(wrapped_query));
                    if let Ok(Json(mut res)) = res {
                        log_info(crate::tools::log::LogServiceType::Plugin, format!("Lookup metadata result from plugin {}", plugin_with_cred.plugin.name));
                        results.append(&mut res);
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request lookup_metadata {} {:?}", code, error))
                        }
                    }
                }
            }
        }
        Ok(results)
    }

    pub async fn lookup_metadata_grouped(&self, query: RsLookupQuery, plugins: Vec<PluginWithCredential>, target: Option<PluginTarget>) -> RsResult<HashMap<String, Vec<RsLookupMetadataResultWithImages>>> {
        let plugins = Self::filter_plugins_by_target(plugins, &target, None)?;
        let mut results: HashMap<String, Vec<RsLookupMetadataResultWithImages>> = HashMap::new();
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();

                if plugin.infos.capabilities.contains(&PluginType::LookupMetadata) {
                    let wrapped_query = RsLookupWrapper {
                        query: query.clone(),
                        credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                        params: plugin_with_cred.credential.as_ref()
                            .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
                    };
                    let res = plugin_m.call_get_error_code::<Json<RsLookupWrapper>, Json<Vec<RsLookupMetadataResultWithImages>>>("lookup_metadata", Json(wrapped_query));
                    if let Ok(Json(res)) = res {
                        log_info(crate::tools::log::LogServiceType::Plugin, format!("Lookup metadata result from plugin {}", plugin_with_cred.plugin.name));
                        results.entry(plugin_with_cred.plugin.name.clone()).or_default().extend(res);
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request lookup_metadata {} {:?}", code, error))
                        }
                    }
                }
            }
        }
        Ok(results)
    }

    pub async fn lookup_metadata_stream(self: &std::sync::Arc<Self>, query: RsLookupQuery, plugins: Vec<PluginWithCredential>, target: Option<PluginTarget>) -> RsResult<tokio::sync::mpsc::Receiver<Vec<RsLookupMetadataResultWithImages>>> {
        let plugins = Self::filter_plugins_by_target(plugins, &target, None)?;
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let manager = self.clone();
        tokio::spawn(async move {
            for plugin_with_cred in plugins {
                let call_result = {
                    let plugins_guard = manager.plugins.read().await;
                    if let Some(plugin) = plugins_guard.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                        if plugin.infos.capabilities.contains(&PluginType::LookupMetadata) {
                            let mut plugin_m = plugin.plugin.lock().unwrap();
                            let wrapped_query = RsLookupWrapper {
                                query: query.clone(),
                                credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                                params: plugin_with_cred.credential.as_ref()
                                    .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
                            };
                            let res = plugin_m.call_get_error_code::<Json<RsLookupWrapper>, Json<Vec<RsLookupMetadataResultWithImages>>>("lookup_metadata", Json(wrapped_query));
                            match res {
                                Ok(Json(res)) => {
                                    log_info(crate::tools::log::LogServiceType::Plugin, format!("Lookup metadata stream result from plugin {}", plugin_with_cred.plugin.name));
                                    Some(Ok(res))
                                }
                                Err((error, code)) => {
                                    if code != 404 {
                                        log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request lookup_metadata {} {:?}", code, error))
                                    }
                                    Some(Err(()))
                                }
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(Ok(res)) = call_result {
                    if tx.send(res).await.is_err() {
                        break;
                    }
                }
            }
        });
        Ok(rx)
    }

    pub async fn lookup_metadata_stream_grouped(self: &std::sync::Arc<Self>, query: RsLookupQuery, plugins: Vec<PluginWithCredential>, target: Option<PluginTarget>) -> RsResult<tokio::sync::mpsc::Receiver<(String, Vec<RsLookupMetadataResultWithImages>)>> {
        let plugins = Self::filter_plugins_by_target(plugins, &target, None)?;
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let manager = self.clone();
        tokio::spawn(async move {
            for plugin_with_cred in plugins {
                let call_result = {
                    let plugins_guard = manager.plugins.read().await;
                    if let Some(plugin) = plugins_guard.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                        if plugin.infos.capabilities.contains(&PluginType::LookupMetadata) {
                            let mut plugin_m = plugin.plugin.lock().unwrap();
                            let wrapped_query = RsLookupWrapper {
                                query: query.clone(),
                                credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                                params: plugin_with_cred.credential.as_ref()
                                    .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
                            };
                            let res = plugin_m.call_get_error_code::<Json<RsLookupWrapper>, Json<Vec<RsLookupMetadataResultWithImages>>>("lookup_metadata", Json(wrapped_query));
                            match res {
                                Ok(Json(res)) => {
                                    log_info(crate::tools::log::LogServiceType::Plugin, format!("Lookup metadata stream result from plugin {}", plugin_with_cred.plugin.name));
                                    Some(Ok((plugin_with_cred.plugin.name.clone(), res)))
                                }
                                Err((error, code)) => {
                                    if code != 404 {
                                        log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request lookup_metadata {} {:?}", code, error))
                                    }
                                    Some(Err(()))
                                }
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(Ok(res)) = call_result {
                    if tx.send(res).await.is_err() {
                        break;
                    }
                }
            }
        });
        Ok(rx)
    }

    /// Check if a request can be played/downloaded instantly without needing to add to service first
    pub async fn check_instant(&self, request: RsRequest, plugins: Vec<PluginWithCredential>, target: Option<PluginTarget>) -> RsResult<Option<bool>> {
        let plugins = Self::filter_plugins_by_target(plugins, &target, None)?;
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                if plugin.infos.capabilities.contains(&PluginType::Request) {
                    let req = RsRequestPluginRequest {
                        request: request.clone(),
                        credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                        params: plugin_with_cred.credential.as_ref()
                            .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
                    };
                    let res = plugin_m.call_get_error_code::<Json<RsRequestPluginRequest>, Json<bool>>("check_instant", Json(req));
                    if let Ok(Json(instant)) = res {
                        return Ok(Some(instant));
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error check_instant: {} {:?}", code, error));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Add a request for processing (RequireAdd status)
    /// Returns the plugin_id and the response from the plugin
    pub async fn request_add(&self, request: RsRequest, plugins: Vec<PluginWithCredential>, target: Option<PluginTarget>) -> RsResult<Option<(String, RsRequestAddResponse)>> {
        let plugins = Self::filter_plugins_by_target(plugins, &target, Some(&request))?;
        for plugin_with_cred in plugins.iter() {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                if plugin.infos.capabilities.contains(&PluginType::Request) {
                    let req = RsRequestPluginRequest {
                        request: request.clone(),
                        credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                        params: plugin_with_cred.credential.as_ref()
                            .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
                    };
                    let res = plugin_m.call_get_error_code::<Json<RsRequestPluginRequest>, Json<RsRequestAddResponse>>("request_add", Json(req));
                    if let Ok(Json(mut response)) = res {
                        // Convert relative ETA (ms) to absolute UTC timestamp (ms)
                        if let Some(relative_eta) = response.eta {
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as i64)
                                .unwrap_or(0);
                            response.eta = Some(now_ms + relative_eta);
                        }
                        return Ok(Some((plugin_with_cred.plugin.id.clone(), response)));
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request_add: {} {:?}", code, error));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Get progress of a processing task
    pub async fn get_processing_progress(&self, processing_id: &str, plugin_with_cred: &PluginWithCredential) -> RsResult<RsProcessingProgress> {
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            let req = RsProcessingActionRequest {
                processing_id: processing_id.to_string(),
                credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                params: plugin_with_cred.credential.as_ref()
                    .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
            };
            let res = plugin_m.call_get_error_code::<Json<RsProcessingActionRequest>, Json<RsProcessingProgress>>("get_progress", Json(req));
            match res {
                Ok(Json(mut progress)) => {
                    // Convert relative ETA (ms) to absolute UTC timestamp (ms)
                    if let Some(relative_eta) = progress.eta {
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        progress.eta = Some(now_ms + relative_eta);
                    }
                    Ok(progress)
                },
                Err((error, code)) => Err(Error::Error(format!("Plugin error {}: {}", code, error))),
            }
        } else {
            Err(Error::NotFound(format!("Plugin not found: {}", plugin_with_cred.plugin.path)))
        }
    }

    /// Pause a processing task
    pub async fn pause_processing(&self, processing_id: &str, plugin_with_cred: &PluginWithCredential) -> RsResult<()> {
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            let req = RsProcessingActionRequest {
                processing_id: processing_id.to_string(),
                credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                params: plugin_with_cred.credential.as_ref()
                    .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
            };
            let res = plugin_m.call_get_error_code::<Json<RsProcessingActionRequest>, ()>("pause", Json(req));
            match res {
                Ok(()) => Ok(()),
                Err((error, code)) => Err(Error::Error(format!("Plugin error {}: {}", code, error))),
            }
        } else {
            Err(Error::NotFound(format!("Plugin not found: {}", plugin_with_cred.plugin.path)))
        }
    }

    /// Remove/cancel a processing task
    pub async fn remove_processing(&self, processing_id: &str, plugin_with_cred: &PluginWithCredential) -> RsResult<()> {
        if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            let req = RsProcessingActionRequest {
                processing_id: processing_id.to_string(),
                credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                params: plugin_with_cred.credential.as_ref()
                    .and_then(|c| serde_json::from_value(c.settings.clone()).ok()),
            };
            let res = plugin_m.call_get_error_code::<Json<RsProcessingActionRequest>, ()>("remove", Json(req));
            match res {
                Ok(()) => Ok(()),
                Err((error, code)) => Err(Error::Error(format!("Plugin error {}: {}", code, error))),
            }
        } else {
            Err(Error::NotFound(format!("Plugin not found: {}", plugin_with_cred.plugin.path)))
        }
    }
}