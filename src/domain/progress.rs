use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, PartialOrd, Default)]
#[serde(rename_all = "snake_case")] 
pub struct RsProgress {
    pub id: String,
	pub total: Option<u64>,
    pub current: Option<u64>,
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