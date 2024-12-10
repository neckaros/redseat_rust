use std::default;

use rs_plugin_common_interfaces::MediaType;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")] 
pub struct Watched {
    #[serde(rename = "type")]
    pub kind: MediaType,
    pub id: String,
    pub user_ref: Option<String>,
    pub date: i64,
    pub modified: u64
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")] 
pub struct WatchedForAdd {
    #[serde(rename = "type")]
    pub kind: MediaType,
    pub id: String,
    pub date: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")] 
pub struct WatchedLight {
    pub date: i64
}
