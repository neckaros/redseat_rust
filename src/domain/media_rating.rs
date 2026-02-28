use rs_plugin_common_interfaces::ElementType;
use serde::{Deserialize, Serialize};



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsMediaRating {
    #[serde(rename = "type", default)]
    pub kind: ElementType,
    #[serde(alias = "mediaRef")]
    pub ref_id: String,
	pub user_ref: String,
    pub rating: f64,
    pub modified: i64
}
