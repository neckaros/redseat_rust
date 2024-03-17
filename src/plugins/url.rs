use extism::convert::Json;
use rs_plugin_common_interfaces::PluginType;

use crate::domain::rs_link::RsLink;

use super::PluginManager;



impl PluginManager {

    pub fn parse(&self, url: String) -> Option<RsLink>{
        for plugin in &self.plugins {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            if plugin.infos.kind == PluginType::UrlParser {
                let res = plugin_m.call::<&str, Json<RsLink>>("parse", &url);
                if let Ok(Json(res)) = res {
                    return Some(res)
                }
                
            }
        }
        None
    }
    pub fn expand(&self, link: RsLink) -> Option<String>{
        for plugin in &self.plugins {
            let mut plugin_m = plugin.plugin.lock().unwrap();
            if plugin.infos.kind == PluginType::UrlParser {
                let res = plugin_m.call::<Json<RsLink>, &str>("expand", Json(link.clone()));
                if let Ok(res) = res {
                    return Some(res.to_string())
                }
                
            }
        }
        None
    }
}