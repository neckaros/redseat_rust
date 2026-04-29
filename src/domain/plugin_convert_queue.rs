use rs_plugin_common_interfaces::video::VideoConvertRequest;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Display, EnumString, Default)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum PluginConvertQueueStatus {
    #[default]
    Queued,
    Submitted,
    Downloading,
    Processing,
    Completed,
    Failed,
    Canceled,
}

impl PluginConvertQueueStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            PluginConvertQueueStatus::Submitted
                | PluginConvertQueueStatus::Downloading
                | PluginConvertQueueStatus::Processing
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            PluginConvertQueueStatus::Completed
                | PluginConvertQueueStatus::Failed
                | PluginConvertQueueStatus::Canceled
        )
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginConvertQueueItem {
    pub id: String,
    pub plugin_id: String,
    pub library_id: String,
    pub media_id: String,
    pub filename: String,
    pub request: VideoConvertRequest,
    pub status: PluginConvertQueueStatus,
    pub plugin_job_id: Option<String>,
    pub progress: f64,
    pub converted_id: Option<String>,
    pub error: Option<String>,
    pub requested_by: Option<String>,
    pub modified: i64,
    pub added: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PluginConvertQueueForInsert {
    pub id: String,
    pub plugin_id: String,
    pub library_id: String,
    pub media_id: String,
    pub filename: String,
    pub request: VideoConvertRequest,
    pub requested_by: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginConvertQueueForUpdate {
    pub status: Option<PluginConvertQueueStatus>,
    pub plugin_job_id: Option<String>,
    pub progress: Option<f64>,
    pub converted_id: Option<String>,
    pub error: Option<String>,
}
