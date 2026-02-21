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
pub use rs_plugin_common_interfaces::domain::media::{FileEpisode, FileType, Media, MediaForUpdate, DEFAULT_MIME, RsGpsPosition, MediaItemReference};
pub use rs_plugin_common_interfaces::domain::ItemWithRelations;

use crate::{domain::backup::BackupFile, plugins::sources::SourceRead};

use super::{people::FaceEmbedding, progress::RsProgress, ElementAction};

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
            people: None,
            tags: None,
            long: value.long,
            lat: value.lat,
            created: value.created,
            origin: value.origin,
            series: None,
            original_hash: value.md5,
            original_id: Some(value.original_id.unwrap_or(value.id)),
            book: None,
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MediaWithAction {
    pub action: ElementAction,
    pub media: ItemWithRelations<Media>,
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
