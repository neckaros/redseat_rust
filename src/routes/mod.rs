use rs_plugin_common_interfaces::{
    lookup::{RsLookupMatchType, RsLookupMetadataResults},
    request::{RsGroupDownload, RsRequest},
    ImageType,
};
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
pub mod upload_keys;
pub mod users;

pub mod books;
pub mod channels;
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SseLookupSearchEvent<'a> {
    pub source_id: &'a str,
    pub source_name: &'a str,
    /// Legacy flattened results. This must remain unchanged so older clients
    /// continue to see every request in a grouped download.
    pub results: &'a [SseLookupSearchResult],
    /// Complete download groups for clients that understand grouped results.
    pub downloads: &'a [RsGroupDownload],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SseLookupSearchResult {
    pub request: RsRequest,
    pub match_type: Option<RsLookupMatchType>,
}

impl SseLookupSearchResult {
    pub fn from_groups(groups: &[RsGroupDownload]) -> Vec<Self> {
        groups
            .iter()
            .flat_map(|download| {
                download
                    .requests
                    .iter()
                    .cloned()
                    .map(|request| SseLookupSearchResult {
                        request,
                        match_type: download.match_type.clone(),
                    })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{SseLookupSearchEvent, SseLookupSearchResult};
    use rs_plugin_common_interfaces::request::{RsGroupDownload, RsRequest};

    #[test]
    fn lookup_stream_keeps_legacy_results_flattened() {
        let first = RsRequest {
            url: "https://example.test/page-1.jpg".to_string(),
            ..Default::default()
        };
        let second = RsRequest {
            url: "https://example.test/page-2.jpg".to_string(),
            ..Default::default()
        };
        let group = RsGroupDownload {
            group: true,
            group_filename: Some("Chapter 1".to_string()),
            requests: vec![first.clone(), second.clone()],
            ..Default::default()
        };

        let results = SseLookupSearchResult::from_groups(std::slice::from_ref(&group));

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].request, first);
        assert_eq!(results[1].request, second);
    }

    #[test]
    fn lookup_stream_serializes_complete_downloads_alongside_legacy_results() {
        let group = RsGroupDownload {
            group: true,
            group_filename: Some("Chapter 1".to_string()),
            requests: vec![RsRequest {
                url: "https://example.test/page-1.jpg".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let results = SseLookupSearchResult::from_groups(std::slice::from_ref(&group));
        let event = SseLookupSearchEvent {
            source_id: "plugin-id",
            source_name: "Plugin name",
            results: &results,
            downloads: std::slice::from_ref(&group),
        };
        let value = serde_json::to_value(event).expect("serialize lookup event");

        assert_eq!(value["results"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["downloads"][0]["group"], true);
        assert_eq!(value["downloads"][0]["groupFilename"], "Chapter 1");
        assert_eq!(
            value["downloads"][0]["requests"].as_array().map(Vec::len),
            Some(1)
        );
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

#[derive(Debug, Deserialize)]
pub struct RatingUpdateBody {
    pub rating: f64,
}
