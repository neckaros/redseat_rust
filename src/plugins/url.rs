use std::collections::HashMap;

use extism::convert::Json;
use futures::future::ok;
use http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use plugin_request_interfaces::{RsRequest, RsRequestStatus, RsRequestPluginRequest};
use rs_plugin_common_interfaces::{PluginCredential, PluginType};
use rs_plugin_lookup_interfaces::{RsLookupQuery, RsLookupResult, RsLookupWrapper};

use crate::{domain::{plugin::PluginWithCredential, progress::RsProgressCallback}, error::RsResult, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::{array_tools::AddOrSetArray, file_tools::get_mime_from_filename, http_tools::{extract_header, guess_filename, parse_content_disposition}, log::log_error, video_tools::ytdl::YydlContext}, Error};

use super::{sources::SourceRead, PluginManager};

use rs_plugin_url_interfaces::RsLink;

impl PluginManager {

    pub async fn parse(&self, url: String, plugins: impl Iterator<Item = PluginWithCredential>) -> Option<RsLink>{
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                if plugin.infos.kind == PluginType::UrlParser {
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
                if plugin.infos.kind == PluginType::UrlParser {
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

    pub async fn request(&self, mut request: RsRequest, savable: bool, plugins: impl Iterator<Item = PluginWithCredential>, progress: RsProgressCallback) -> RsResult<SourceRead> {
        let client = reqwest::Client::new();
        let r = client.head(&request.url).send().await;
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
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                if plugin.infos.kind == PluginType::Request {
                    let req = RsRequestPluginRequest {
                        request: request.clone(),
                        credential: plugin_with_cred.credential.clone().and_then(|p| Some(PluginCredential::from(p))),
                        savable
                    };
                    //println!("request {}", serde_json::to_string(&req).unwrap());
                    let res = plugin_m.call_get_error_code::<Json<RsRequestPluginRequest>, Json<RsRequest>>("process", Json(req));
                    if let Ok(Json(mut res)) = res {
                        if res.mime.is_none() {
                            res.mime = get_mime_from_filename(&res.url);
                        }
                        return Ok(SourceRead::Request(res));
                    } else if let Err((error, code)) = res {
                        if code != 404 {
                            log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request {} {:?}", code, error))
                        }
                    }
                    
                }
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

    pub async fn lookup(&self, query: RsLookupQuery, plugins: impl Iterator<Item = PluginWithCredential>) -> RsResult<Vec<RsLookupResult>> {
        let mut results = vec![];
        for plugin_with_cred in plugins {
            if let Some(plugin) = self.plugins.read().await.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                
                println!("PLUGIN {}", plugin_with_cred.plugin.name);
                if plugin.infos.kind == PluginType::Lookup {
                    let wrapped_query = RsLookupWrapper {
                        query: query.clone(),
                        credential: plugin_with_cred.credential.clone().and_then(|p| Some(PluginCredential::from(p))),
                        params: None,
                    };
                    //println!("request {}", serde_json::to_string(&wrapped_query).unwrap());
                    let res = plugin_m.call_get_error_code::<Json<RsLookupWrapper>, Json<RsLookupResult>>("process", Json(wrapped_query));
                    if let Ok(Json(res)) = res {
                        results.push(res);
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