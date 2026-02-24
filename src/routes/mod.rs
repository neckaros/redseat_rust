use rs_plugin_common_interfaces::{lookup::RsLookupMetadataResults, ImageType};
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
pub mod search;
pub mod series;
pub mod tags;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SseSearchEvent<'a> {
    pub source_id: &'a str,
    pub source_name: &'a str,
    #[serde(flatten)]
    pub data: &'a RsLookupMetadataResults,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultGroup {
    pub source_id: String,
    pub source_name: String,
    #[serde(flatten)]
    pub data: RsLookupMetadataResults,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery<T> {
    #[serde(flatten)]
    pub lookup: T,
    pub source: Option<String>,
}

impl<T> SearchQuery<T> {
    pub fn sources(&self) -> Option<Vec<String>> {
        self.source.as_deref().map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
    }
}

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
