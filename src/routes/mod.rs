use rs_plugin_common_interfaces::ImageType;
use serde::{Deserialize, Serialize};

use crate::tools::image_tools::ImageSize;

pub mod backups;
pub mod credentials;
pub mod infos;
pub mod libraries;
pub mod mw_auth;
pub mod mw_range;
pub mod ping;
pub mod plugins;
pub mod sse;
pub mod users;

pub mod books;
pub mod episodes;
pub mod library_plugins;
pub mod medias;
pub mod movies;
pub mod people;
pub mod series;
pub mod tags;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImageRequestOptions {
    size: Option<ImageSize>,
    #[serde(rename = "type")]
    kind: Option<ImageType>,
    #[serde(default)]
    defaulting: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImageUploadOptions {
    #[serde(rename = "type")]
    kind: ImageType,
}
