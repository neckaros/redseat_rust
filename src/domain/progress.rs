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
    Analysing,
    Finished,
    Duplicate(String),
    Failed(String),
}

impl RsProgress {
    pub fn percent(&self) -> Option<f32> {
        if let (Some(total), Some(current)) = (self.total, self.current) {
            if total == 0 {
                return None;
            }
            Some(current as f32 / total as f32)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_progress_serializes_as_failed_payload() {
        let progress = RsProgress {
            id: "upload-1".to_string(),
            total: Some(1),
            current: Some(1),
            filename: Some("photo.jpg".to_string()),
            kind: RsProgressType::Failed("network error".to_string()),
        };

        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains("\"failed\":\"network error\""));
    }

    #[test]
    fn percent_ignores_zero_total() {
        let progress = RsProgress {
            total: Some(0),
            current: Some(0),
            ..Default::default()
        };

        assert_eq!(progress.percent(), None);
    }
}
