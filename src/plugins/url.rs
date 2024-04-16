use std::collections::HashMap;

use async_recursion::async_recursion;
use extism::convert::Json;
use futures::future::ok;
use http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use rs_plugin_common_interfaces::{lookup::{RsLookupQuery, RsLookupSourceResult, RsLookupWrapper}, request::{RsRequest, RsRequestPluginRequest, RsRequestStatus}, url::RsLink, PluginCredential, PluginType};

use crate::{domain::{plugin::PluginWithCredential, progress::RsProgressCallback}, error::RsResult, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::{array_tools::AddOrSetArray, file_tools::get_mime_from_filename, http_tools::{extract_header, guess_filename, parse_content_disposition}, log::log_error, video_tools::ytdl::YydlContext}, Error};

use super::{sources::{RsRequestHeader, SourceRead}, PluginManager};


impl PluginManager {

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
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request {} {:?}", code, error))
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
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request {} {:?}", code, error))
                        }
                    }
                    
                }
            }
        }
        None
    }



    pub async fn fill_infos(&self, request: &mut RsRequest) {
        let ctx = YydlContext::new().await;
        if let Ok(ctx) = ctx {
            let video = ctx.request_infos(&request).await;
            if let Ok(Some(video)) = video {
                if let Some(tags) = video.tags {
                    request.tags.add_or_set(tags);
                }
                if let Some(person) = video.uploader {
                    request.people.add_or_set(vec![person]);
                }
                if let Some(description) = video.description {
                    if request.description == None {
                        request.description = Some(description);
                    }
                    
                }
            }
        }
        

    }

    #[async_recursion]
    pub async fn request(&self, mut request: RsRequest, _savable: bool, plugins: Vec<PluginWithCredential>, progress: RsProgressCallback) -> RsResult<SourceRead> {
        let initial_request = request.clone();
        
        let client = reqwest::Client::new();
        let r = client.head(&request.url).add_request_headers(&request, &None)?;

        let r = r.send().await;
        if let Ok(heads) = r {
            let headers = heads.headers();
            if let Some(mime) = extract_header(headers, CONTENT_TYPE) {
                request.mime = Some(mime.to_string())
            }
            if let Some(size) = extract_header(headers, CONTENT_LENGTH).and_then(|c| c.parse::<u64>().ok()) {
                request.size = Some(size);
            }
            if let Some(filename) = extract_header(headers, CONTENT_DISPOSITION).and_then(parse_content_disposition) {
                println!("dispo {}", filename);
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
                    };
                    //println!("request {}", serde_json::to_string(&req).unwrap());
                    let res = plugin_m.call_get_error_code::<Json<RsRequestPluginRequest>, Json<RsRequest>>("process", Json(req));
                    if let Ok(Json(mut res)) = res {
                        if res.mime.is_none() {
                            res.mime = get_mime_from_filename(&res.url);
                        }

                        processed_request = Some(res);
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request {} {:?}", code, error))
                        }
                    }
                    
                }
            }
        }
        if let Some(processed) = processed_request {

            if processed.status == RsRequestStatus::Intermediate && processed != initial_request {
                println!("recurs");
                let recursed = self.request(processed, false, plugins, progress).await?;
                return Ok(recursed);
            } else {
                return Ok(SourceRead::Request(processed));
            }
        }
        if request.status == RsRequestStatus::NeedParsing || request.url.ends_with(".m3u8") || request.mime.as_deref().unwrap_or("no") == "application/vnd.apple.mpegurl" {
            let ctx = YydlContext::new().await?;
            let result = ctx.request(&request, progress).await?;

            return Ok(result);

        } else {
            request.status = RsRequestStatus::FinalPublic;
        }
        
        Ok(SourceRead::Request(request))
    }

    pub async fn request_permanent(&self, request: RsRequest, plugins: impl Iterator<Item = PluginWithCredential>, _progress: RsProgressCallback) -> RsResult<Option<RsRequest>> {
        if request.permanent {
            Ok(Some(request))
        } else {
            for plugin_with_cred in plugins {
                if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                    let mut plugin_m = plugin.plugin.lock().unwrap();
                    if plugin.infos.capabilities.contains(&PluginType::Request) {
                        let req = RsRequestPluginRequest {
                            request: request.clone(),
                            credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                        };
                        //println!("request {}", serde_json::to_string(&req).unwrap());
                        let res = plugin_m.call_get_error_code::<Json<RsRequestPluginRequest>, Json<RsRequest>>("request_permanent", Json(req));
                        if let Ok(Json(mut res)) = res {
                            if res.mime.is_none() {
                                res.mime = get_mime_from_filename(&res.url);
                            }
                            if res.permanent {
                                return Ok(Some(res));
                            }
                            
                        } else if let Err((error, code)) = res {
                            if code != 404 {
                                log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request {} {:?}", code, error))
                            }
                        }
                        
                    }
                }
            }
            Ok(None)
        }
    }

    pub async fn lookup(&self, query: RsLookupQuery, plugins: impl Iterator<Item = PluginWithCredential>) -> RsResult<Vec<RsRequest>> {
        let mut results = vec![];
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                
                println!("PLUGIN {}", plugin_with_cred.plugin.name);
                if plugin.infos.capabilities.contains(&PluginType::Lookup) {
                    let wrapped_query = RsLookupWrapper {
                        query: query.clone(),
                        credential: plugin_with_cred.credential.clone().map(PluginCredential::from),
                        params: None,
                    };
                    //println!("request {}", serde_json::to_string(&wrapped_query).unwrap());
                    let res = plugin_m.call_get_error_code::<Json<RsLookupWrapper>, Json<RsLookupSourceResult>>("lookup", Json(wrapped_query));
                    if let Ok(Json(res)) = res {
                        match res {
                            RsLookupSourceResult::Requests(mut request) => results.append(&mut request),
                            _ => (),
                        }
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request {} {:?}", code, error))
                        }
                    }
                    
                }
            }
        }
        Ok(results)
    }
}