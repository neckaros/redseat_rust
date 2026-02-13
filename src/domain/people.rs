use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ElementAction;
use rs_plugin_common_interfaces::{url::RsLink, Gender};

pub use rs_plugin_common_interfaces::domain::person::Person;
pub use rs_plugin_common_interfaces::domain::media::{FaceBBox, FaceEmbedding};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PeopleMessage {
    pub library: String,
    pub people: Vec<PersonWithAction>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PersonWithAction {
    pub action: ElementAction,
    pub person: Person,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UnassignedFace {
    pub id: String,
    pub embedding: Vec<f32>,
    pub media_ref: String,
    pub bbox: FaceBBox,
    pub confidence: f32,
    pub pose: Option<(f32, f32, f32)>,
    pub cluster_id: Option<String>,
    pub created: i64,
}
