use rs_plugin_common_interfaces::request::{RsProcessingStatus, RsRequest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsRequestProcessing {
    pub id: String,
    pub processing_id: String,
    pub plugin_id: String,
    pub progress: u32,
    pub status: RsProcessingStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// UTC timestamp (milliseconds) for estimated completion
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_ref: Option<String>,
    /// The original RsRequest used to create this processing task
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_request: Option<RsRequest>,
    pub modified: i64,
    pub added: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsRequestProcessingForInsert {
    pub id: String,
    pub processing_id: String,
    pub plugin_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_ref: Option<String>,
    /// The original RsRequest used to create this processing task
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_request: Option<RsRequest>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsRequestProcessingForUpdate {
    pub progress: Option<u32>,
    pub status: Option<RsProcessingStatus>,
    pub error: Option<String>,
    pub eta: Option<i64>,
}
