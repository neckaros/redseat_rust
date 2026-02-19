use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use rs_plugin_common_interfaces::domain::tag::{Tag, TagForUpdate};


use super::ElementAction;



#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct TagMessage {
    pub library: String,
    pub tags: Vec<TagWithAction>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct TagWithAction {
    pub action: ElementAction,
    pub tag: Tag
}
