use std::default;

use rs_plugin_common_interfaces::MediaType;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")] 
pub struct ViewProgress {
    #[serde(rename = "type")]
    pub kind: MediaType,
    pub id: String,
    pub user_ref: String,
    pub progress: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub modified: u64
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")] 
pub struct ViewProgressForAdd {
    #[serde(rename = "type")]
    pub kind: MediaType,
    pub id: String,
    pub parent: Option<String>,
    pub progress: u64
}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")] 
pub struct ViewProgressLigh {
    pub progress: u64
}
