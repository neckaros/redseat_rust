use std::str::FromStr;

use nanoid::nanoid;
use rs_plugin_common_interfaces::{
    request::{RsCookie, RsRequest, RsRequestStatus},
    url::RsLink,
    video::{RsVideoTranscodeStatus, VideoConvertRequest},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;
pub use rs_plugin_common_interfaces::domain::media::{FileEpisode, FileType, Media, RsGpsPosition, MediaItemReference};

use crate::{domain::backup::BackupFile, plugins::sources::SourceRead};

use super::{people::FaceEmbedding, progress::RsProgress, ElementAction};

pub const DEFAULT_MIME: &str = "application/octet-stream";


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaForUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub mimetype: Option<String>,
    pub kind: Option<FileType>,
    pub size: Option<u64>,

    pub md5: Option<String>,

    pub modified: Option<i64>,
    pub created: Option<i64>,

    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<u8>,
    pub color_space: Option<String>,
    pub icc: Option<String>,
    pub mp: Option<u32>,
    pub vcodecs: Option<Vec<String>>,
    pub acodecs: Option<Vec<String>>,
    pub fps: Option<f64>,
    pub bitrate: Option<u64>,
    pub focal: Option<u64>,
    pub iso: Option<u64>,
    pub model: Option<String>,
    pub sspeed: Option<String>,
    pub f_number: Option<f64>,

    pub pages: Option<usize>,

    pub duration: Option<u64>,

    pub progress: Option<usize>,

    pub add_tags: Option<Vec<MediaItemReference>>,
    pub remove_tags: Option<Vec<String>>,
    pub tags_lookup: Option<Vec<String>>,

    pub add_series: Option<Vec<FileEpisode>>,
    pub remove_series: Option<Vec<FileEpisode>>,
    pub series_lookup: Option<Vec<String>>,
    pub season: Option<u32>,
    pub episode: Option<u32>,

    pub add_people: Option<Vec<MediaItemReference>>,
    pub remove_people: Option<Vec<String>>,
    pub people_lookup: Option<Vec<String>>,

    pub long: Option<f64>,
    pub lat: Option<f64>,
    pub gps: Option<String>,

    pub origin: Option<RsLink>,
    pub origin_url: Option<String>,
    #[serde(default)]
    pub ignore_origin_duplicate: bool,

    pub movie: Option<String>,
    pub book: Option<String>,

    pub lang: Option<String>,

    pub rating: Option<u16>,

    pub thumbsize: Option<usize>,
    pub iv: Option<String>,

    pub uploader: Option<String>,
    pub uploadkey: Option<String>,
    pub upload_id: Option<String>,

    pub original_hash: Option<String>,
    pub original_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct MediaForAdd {
    pub source: Option<String>,
    pub name: String,
    pub description: Option<String>,

    #[serde(rename = "type")]
    pub kind: FileType,
    pub mimetype: String,
    pub size: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,

    pub rating: Option<u8>,
    pub md5: Option<String>,

    pub width: Option<usize>,
    pub height: Option<usize>,
    pub phash: Option<String>,
    pub thumbhash: Option<String>,
    pub focal: Option<u64>,
    pub iso: Option<u64>,
    pub color_space: Option<String>,
    pub icc: Option<String>,
    pub mp: Option<u32>,
    pub sspeed: Option<String>,
    pub f_number: Option<f64>,
    pub orientation: Option<usize>,

    pub duration: Option<usize>,
    pub acodecs: Option<Vec<String>>,
    pub achan: Option<Vec<usize>>,
    pub vcodecs: Option<Vec<String>>,
    pub fps: Option<f64>,
    pub bitrate: Option<u64>,

    pub long: Option<f64>,
    pub lat: Option<f64>,
    pub model: Option<String>,

    pub pages: Option<usize>,

    pub progress: Option<usize>,
    pub tags: Option<Vec<String>>,
    pub series: Option<Vec<FileEpisode>>,
    pub people: Option<Vec<String>>,
    pub thumb: Option<String>,
    pub thumbv: Option<usize>,
    pub thumbsize: Option<usize>,
    pub iv: Option<String>,
    pub origin: Option<RsLink>,
    pub movie: Option<String>,
    pub book: Option<String>,
    pub lang: Option<String>,
    pub uploader: Option<String>,
    pub uploadkey: Option<String>,

    pub original_hash: Option<String>,
    pub original_id: Option<String>,

    pub created: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MediaForInsert {
    pub id: String,
    pub media: MediaForAdd,
}

impl MediaForAdd {
    pub fn into_insert(self) -> MediaForInsert {
        MediaForInsert {
            id: nanoid!(),
            media: self,
        }
    }
    pub fn into_insert_with_id(self, media_id: String) -> MediaForInsert {
        MediaForInsert {
            id: media_id,
            media: self,
        }
    }
}

impl From<Media> for MediaForAdd {
    fn from(value: Media) -> Self {
        MediaForAdd {
            name: value.name,
            description: value.description,
            people: value
                .people
                .map(|e| e.iter().map(|p| p.id.to_string()).collect::<Vec<String>>()),
            tags: value
                .tags
                .map(|e| e.iter().map(|p| p.id.to_string()).collect::<Vec<String>>()),
            long: value.long,
            lat: value.lat,
            created: value.created,
            origin: value.origin,
            series: value.series,
            original_hash: value.md5,
            original_id: Some(value.original_id.unwrap_or(value.id)),
            book: value.book,
            ..Default::default()
        }
    }
}

impl From<Media> for MediaForUpdate {
    fn from(value: Media) -> Self {
        MediaForUpdate {
            description: value.description,
            add_people: value.people,
            add_tags: value.tags,
            long: value.long,
            lat: value.lat,
            created: value.created,
            origin: value.origin,
            add_series: value.series,
            pages: value.pages,
            original_hash: value.original_hash.or(value.md5),
            original_id: Some(value.original_id.unwrap_or(value.id)),
            book: value.book,
            ..Default::default()
        }
    }
}

impl From<RsRequest> for MediaForUpdate {
    fn from(value: RsRequest) -> Self {
        // Build add_series from albums (first entry) + season + episode if albums are provided
        let (add_series, season, episode) = if let Some(albums) = &value.albums {
            if let Some(serie_id) = albums.first() {
                // Have direct serie ID - use add_series, don't need season/episode at top level
                (
                    Some(vec![FileEpisode {
                        id: serie_id.clone(),
                        season: value.season,
                        episode: value.episode,
                        episode_to: None,
                    }]),
                    None,
                    None,
                )
            } else {
                (None, value.season, value.episode)
            }
        } else {
            // No direct album IDs - keep season/episode for series_lookup pairing
            (None, value.season, value.episode)
        };

        MediaForUpdate {
            name: value.filename_or_extract_from_url(),
            description: value.description,
            ignore_origin_duplicate: value.ignore_origin_duplicate,
            size: value.size,
            // Use the new lookup fields for database text search
            people_lookup: value.people_lookup,
            tags_lookup: value.tags_lookup,
            series_lookup: value.albums_lookup,
            add_series,
            movie: value.movie,
            season,
            episode,
            ..Default::default()
        }
    }
}

impl From<&SourceRead> for MediaForUpdate {
    fn from(value: &SourceRead) -> Self {
        match value {
            SourceRead::Stream(stream) => MediaForUpdate {
                name: stream.name.clone(),
                mimetype: stream.mime.clone(),
                size: stream.size.clone(),
                ..Default::default()
            },
            SourceRead::Request(r) => MediaForUpdate {
                name: r.filename.clone(),
                mimetype: r.mime.clone(),
                size: r.size.clone(),
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MediaWithAction {
    pub action: ElementAction,
    pub media: Media,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MediasMessage {
    pub library: String,
    pub medias: Vec<MediaWithAction>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct UploadProgressMessage {
    pub library: String,
    pub progress: RsProgress,
    pub remaining_secondes: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConvertProgress {
    pub id: String,
    pub filename: String,
    pub converted_id: Option<String>,
    pub done: bool,
    pub percent: f64,
    pub status: RsVideoTranscodeStatus,
    pub estimated_remaining_seconds: Option<u64>,
    pub request: Option<VideoConvertRequest>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConvertMessage {
    pub library: String,
    pub progress: ConvertProgress,
}
