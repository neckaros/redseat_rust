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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WatchedForDelete {
    #[serde(rename = "type")]
    pub kind: MediaType,
    /// Multiple possible IDs to try (imdb, trakt, tmdb, local, etc.)
    pub ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Unwatched {
    #[serde(rename = "type")]
    pub kind: MediaType,
    /// All possible IDs for this content (imdb, trakt, tmdb, local, etc.)
    pub ids: Vec<String>,
    pub user_ref: Option<String>,
    pub modified: u64,
}
