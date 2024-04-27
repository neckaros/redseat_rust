use std::default;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;

pub type RsProgressCallback = Option<Sender<RsProgress>>;



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, PartialOrd, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsProgress {
    pub id: String,
	pub total: Option<u64>,
    pub current: Option<u64>,
    pub filename: Option<String>,
    #[serde(rename = "type")]
    pub kind: RsProgressType,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, PartialOrd, Default)]
#[serde(rename_all = "camelCase")]
pub enum RsProgressType {
    Download,
    #[default]
    Transfert,
    Finished,
    Duplicate(String),
}

impl RsProgress {
    pub fn percent(&self) -> Option<f32> {
        if let (Some(total), Some(current)) = (self.total, self.current) {
            Some(current as f32 / total as f32)
        } else {
            None
        }
    }
}