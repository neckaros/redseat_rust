use serde::{Deserialize, Serialize};

use super::ElementAction;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tvg_id: Option<String>,
    #[serde(skip_serializing)]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_number: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posterv: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<Vec<ChannelVariant>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChannelVariant {
    pub id: String,
    pub channel_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    pub stream_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChannelForAdd {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tvg_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_number: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChannelForUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tvg_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_number: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChannelVariantForAdd {
    pub channel_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    pub stream_url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChannelWithAction {
    pub action: ElementAction,
    pub channel: Channel,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessage {
    pub library: String,
    pub channels: Vec<ChannelWithAction>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct M3uImportResult {
    pub channels_added: usize,
    pub channels_updated: usize,
    pub channels_removed: usize,
    pub movies_added: usize,
    pub series_added: usize,
    pub episodes_added: usize,
    pub groups_created: usize,
    pub total_parsed: usize,
}
