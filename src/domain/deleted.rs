use std::default;

use rs_plugin_common_interfaces::ElementType;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;

use crate::tools::clock::now;



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsDeleted {
    pub id: String,
	pub date: i64,
    #[serde(rename = "type")]
    pub kind: ElementType,
}

impl RsDeleted {
    pub fn media(id: String) -> Self {
        Self { id, date: now().timestamp_millis(), kind: ElementType::Media }
    }
    pub fn tag(id: String) -> Self {
        Self { id, date: now().timestamp_millis(), kind: ElementType::Tag }
    }
    pub fn person(id: String) -> Self {
        Self { id, date: now().timestamp_millis(), kind: ElementType::Person }
    }
    pub fn episode(id: String) -> Self {
        Self { id, date: now().timestamp_millis(), kind: ElementType::Episode }
    }
    pub fn serie(id: String) -> Self {
        Self { id, date: now().timestamp_millis(), kind: ElementType::Serie }
    }
    pub fn movie(id: String) -> Self {
        Self { id, date: now().timestamp_millis(), kind: ElementType::Movie }
    }
}