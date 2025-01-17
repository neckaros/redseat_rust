use std::default;

use rs_plugin_common_interfaces::ElementType;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;

use crate::tools::clock::now;



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsMediaProgress {
    pub media_ref: String,
	pub user_ref: String,
    pub progress: u64,
    pub modified: i64,
}
