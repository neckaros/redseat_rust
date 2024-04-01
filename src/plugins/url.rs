use extism::convert::Json;
use http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use plugin_request_interfaces::{RsRequest, RsRequestStatus, RsRequestWithCredential};
use rs_plugin_common_interfaces::{PluginCredential, PluginType};

use crate::{domain::{plugin::PluginWithCredential, progress::RsProgressCallback}, error::RsResult, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::{file_tools::get_mime_from_filename, http_tools::{extract_header, guess_filename, parse_content_disposition}, log::log_error, video_tools::ytdl::YydlContext}, Error};

use super::{sources::SourceRead, PluginManager};

use rs_plugin_url_interfaces::RsLink;

impl PluginManager {

    pub fn parse(&self, url: String) -> Option<RsLink>{
        for plugin in &self.plugins {
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
        None
    }
    pub fn expand(&self, link: RsLink) -> Option<String>{
        for plugin in &self.plugins {
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
        None
    }

    pub async fn request(&self, mut request: RsRequest, plugins: impl Iterator<Item = PluginWithCredential>, progress: RsProgressCallback) -> RsResult<SourceRead> {
        println!("Plugins");
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
            if let Some(plugin) = self.plugins.iter().find(|p| p.filename == plugin_with_cred.plugin.path) {
                let mut plugin_m = plugin.plugin.lock().unwrap();
                if plugin.infos.kind == PluginType::Request {
                    let req = RsRequestWithCredential {
                        request: request.clone(),
                        credential: plugin_with_cred.credential.clone().and_then(|p| Some(PluginCredential::from(p)))
                    };
                    let res = plugin_m.call_get_error_code::<Json<RsRequestWithCredential>, Json<RsRequest>>("process", Json(req));
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

    pub fn request_all(&self, request: RsRequest) -> Option<RsRequest>{
        for plugin in &self.plugins {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            if plugin.infos.kind == PluginType::Request {
                let res = plugin_m.call_get_error_code::<Json<RsRequest>, Json<RsRequest>>("process", Json(request.clone()));
                if let Ok(Json(res)) = res {
                    return Some(res);
                } else if let Err((error, code)) = res {
                    if code != 404 {
                        log_error(crate::tools::log::LogServiceType::Plugin, format!("Error request {} {:?}", code, error))
                    }
                }
                
            }
        }
        None
    }
}