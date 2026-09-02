use rs_plugin_common_interfaces::{
    domain::media::FileEpisode,
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
        parse_sources(self.source.as_deref())
    }
}

/// Provider-specific pagination options for media/download lookups.
///
/// Page keys are opaque values interpreted by lookup plugins. Clients should
/// send the key back together with the source that produced it.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LookupPagination {
    pub q: Option<String>,
    pub page_key: Option<String>,
    pub source: Option<String>,
}

impl LookupPagination {
    pub fn query(&self) -> Option<String> {
        self.q
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(str::to_string)
    }

    pub fn resolve(&self) -> crate::Result<(Option<String>, Option<Vec<String>>)> {
        let page_key = self
            .page_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string);
        let sources = parse_sources(self.source.as_deref());

        if page_key.is_some() && sources.as_ref().map(Vec::len) != Some(1) {
            return Err(crate::Error::InvalidParams(
                "pageKey requires exactly one non-empty source".to_string(),
            ));
        }

        Ok((page_key, sources))
    }
}

fn parse_sources(source: Option<&str>) -> Option<Vec<String>> {
    source.and_then(|source| {
        let sources: Vec<_> = source
            .split(',')
            .map(str::trim)
            .filter(|source| !source.is_empty())
            .map(str::to_string)
            .collect();
        (!sources.is_empty()).then_some(sources)
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SseLookupSearchEvent<'a> {
    pub source_id: &'a str,
    pub source_name: &'a str,
    /// Legacy flattened results. This must remain unchanged so older clients
    /// continue to see every request in a grouped download.
    pub results: &'a [SseLookupSearchResult<'a>],
    /// Complete download groups for clients that understand grouped results.
    pub downloads: &'a [RsGroupDownload],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_key: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SseLookupSearchResult<'a> {
    pub request: &'a RsRequest,
    pub match_type: Option<RsLookupMatchType>,
}

impl<'a> SseLookupSearchResult<'a> {
    pub fn from_groups(groups: &'a [RsGroupDownload]) -> Vec<Self> {
        groups
            .iter()
            .flat_map(|download| {
                download
                    .requests
                    .iter()
                    .map(|request| SseLookupSearchResult {
                        request,
                        match_type: download.match_type.clone(),
                    })
            })
            .collect()
    }
}

pub fn bind_downloads_to_movie(downloads: &mut [RsGroupDownload], movie_id: &str) {
    for download in downloads {
        for request in &mut download.requests {
            request.movie = Some(movie_id.to_owned());
        }
        download.infos.get_or_insert_default().movie = Some(movie_id.to_owned());
    }
}

pub fn bind_downloads_to_book(downloads: &mut [RsGroupDownload], book_id: &str) {
    for download in downloads {
        for request in &mut download.requests {
            request.book = Some(FileEpisode {
                id: book_id.to_owned(),
                season: None,
                episode: None,
                episode_to: None,
            });
        }
        download.infos.get_or_insert_default().book = Some(book_id.to_owned());
    }
}

pub fn bind_downloads_to_series(
    downloads: &mut [RsGroupDownload],
    serie_id: &str,
    season: u32,
    episode: Option<u32>,
) {
    for download in downloads {
        for request in &mut download.requests {
            request.albums = Some(vec![serie_id.to_owned()]);
            request.season = Some(season);
            if episode.is_some() {
                request.episode = episode;
            }
        }

        let association = FileEpisode {
            id: serie_id.to_owned(),
            season: Some(season),
            episode: episode.or_else(|| download.requests.first().and_then(|r| r.episode)),
            episode_to: None,
        };
        let associations = download
            .infos
            .get_or_insert_default()
            .add_series
            .get_or_insert_default();
        if let Some(existing) = associations.iter_mut().find(|item| item.id == serie_id) {
            *existing = association;
        } else {
            associations.push(association);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bind_downloads_to_movie, bind_downloads_to_series, LookupPagination, SseLookupSearchEvent,
        SseLookupSearchResult,
    };
    use rs_plugin_common_interfaces::request::{RsGroupDownload, RsRequest};

    #[test]
    fn lookup_pagination_normalizes_page_key_and_source() {
        let pagination: LookupPagination = serde_json::from_value(serde_json::json!({
            "q": "  alternate title  ",
            "pageKey": "  cursor-2  ",
            "source": "  plugin-a  "
        }))
        .expect("deserialize lookup pagination");

        let (page_key, sources) = pagination.resolve().expect("resolve pagination");
        assert_eq!(pagination.query().as_deref(), Some("alternate title"));
        assert_eq!(page_key.as_deref(), Some("cursor-2"));
        assert_eq!(sources, Some(vec!["plugin-a".to_string()]));
    }

    #[test]
    fn lookup_pagination_ignores_an_empty_page_key() {
        let pagination = LookupPagination {
            q: Some("   ".to_string()),
            page_key: Some("   ".to_string()),
            source: None,
        };

        let (page_key, sources) = pagination.resolve().expect("resolve pagination");
        assert_eq!(pagination.query(), None);
        assert_eq!(page_key, None);
        assert_eq!(sources, None);
    }

    #[test]
    fn lookup_pagination_rejects_a_page_key_without_exactly_one_source() {
        for source in [None, Some("   ".to_string()), Some("a,b".to_string())] {
            let pagination = LookupPagination {
                q: None,
                page_key: Some("cursor-2".to_string()),
                source,
            };

            assert!(matches!(
                pagination.resolve(),
                Err(crate::Error::InvalidParams(_))
            ));
        }
    }

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
        assert_eq!(results[0].request, &first);
        assert_eq!(results[1].request, &second);
        assert!(std::ptr::eq(results[0].request, &group.requests[0]));
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
            next_page_key: Some("cursor-2"),
        };
        let value = serde_json::to_value(event).expect("serialize lookup event");

        assert_eq!(value["results"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["downloads"][0]["group"], true);
        assert_eq!(value["downloads"][0]["groupFilename"], "Chapter 1");
        assert_eq!(value["nextPageKey"], "cursor-2");
        assert_eq!(
            value["downloads"][0]["requests"].as_array().map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn lookup_stream_binds_grouped_downloads_to_the_selected_movie() {
        let mut downloads = vec![RsGroupDownload {
            group: true,
            requests: vec![RsRequest::default(), RsRequest::default()],
            ..Default::default()
        }];

        bind_downloads_to_movie(&mut downloads, "movie-id");

        assert!(downloads[0]
            .requests
            .iter()
            .all(|request| request.movie.as_deref() == Some("movie-id")));
        assert_eq!(
            downloads[0]
                .infos
                .as_ref()
                .and_then(|infos| infos.movie.as_deref()),
            Some("movie-id")
        );
    }

    #[test]
    fn lookup_stream_binds_grouped_downloads_to_the_selected_episode() {
        let mut downloads = vec![RsGroupDownload {
            group: true,
            requests: vec![RsRequest::default(), RsRequest::default()],
            ..Default::default()
        }];

        bind_downloads_to_series(&mut downloads, "serie-id", 2, Some(3));

        assert!(downloads[0].requests.iter().all(|request| {
            request.albums.as_deref() == Some(&["serie-id".to_owned()][..])
                && request.season == Some(2)
                && request.episode == Some(3)
        }));
        let association = downloads[0]
            .infos
            .as_ref()
            .and_then(|infos| infos.add_series.as_ref())
            .and_then(|series| series.first())
            .expect("series association");
        assert_eq!(association.id, "serie-id");
        assert_eq!(association.season, Some(2));
        assert_eq!(association.episode, Some(3));
    }

    #[test]
    fn lookup_stream_season_binding_preserves_provider_episode_numbers() {
        let mut downloads = vec![RsGroupDownload {
            group: true,
            requests: vec![RsRequest {
                episode: Some(4),
                ..Default::default()
            }],
            ..Default::default()
        }];

        bind_downloads_to_series(&mut downloads, "serie-id", 2, None);

        assert_eq!(downloads[0].requests[0].episode, Some(4));
        let association = downloads[0]
            .infos
            .as_ref()
            .and_then(|infos| infos.add_series.as_ref())
            .and_then(|series| series.first())
            .expect("series association");
        assert_eq!(association.id, "serie-id");
        assert_eq!(association.season, Some(2));
        assert_eq!(association.episode, Some(4));
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
