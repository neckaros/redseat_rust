use std::str::FromStr;

use nanoid::nanoid;
use rs_plugin_common_interfaces::{request::{RsCookie, RsRequest, RsRequestStatus}, url::RsLink};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;


use crate::{plugins::sources::SourceRead, tools::video_tools::VideoConvertRequest};

use super::{progress::RsProgress, ElementAction};


pub const DEFAULT_MIME: &str = "application/octet-stream";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct FileEpisode {
   pub id: String, 
   #[serde(skip_serializing_if = "Option::is_none")]
   pub season: Option<u32>,
   #[serde(skip_serializing_if = "Option::is_none")]
   pub episode: Option<u32>
}

impl FromStr for FileEpisode {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let splitted: Vec<&str> = s.split("|").collect();
        if splitted.len() == 3 {
            Ok(FileEpisode { id: splitted[0].to_string(), season: splitted[1].parse::<u32>().ok().and_then(|i| if i == 0 {None} else {Some(i)}), episode: splitted[2].parse::<u32>().ok().and_then(|i| if i == 0 {None} else {Some(i)}) })
        } else if splitted.len() == 2 {
            Ok(FileEpisode { id: splitted[0].to_string(), season: splitted[1].parse::<u32>().ok().and_then(|i| if i == 0 {None} else {Some(i)}), episode: None })
        } else {
            Ok(FileEpisode { id: splitted[0].to_string(), season: None, episode: None })
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct MediaItemReference {
   pub id: String,
   #[serde(skip_serializing_if = "Option::is_none")]
   pub conf: Option<u16>
}

impl FromStr for MediaItemReference {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let splitted: Vec<&str> = s.split('|').collect();
        if splitted.len() == 2 {
            Ok(MediaItemReference { id: splitted[0].to_string(), conf: splitted[1].parse::<u16>().ok().and_then(|e| if e == 100 {None} else {Some(e)}) })
        } else {
            Ok(MediaItemReference { id: splitted[0].to_string(), conf: None })
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display,EnumString, Default)]
#[strum(serialize_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub enum FileType {
    Directory,
    Photo,
    Video,
    Archive,
    Album,
    #[default]
    Other
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(rename = "type")]
    pub kind: FileType,
    pub mimetype: String,
    pub size: Option<u64>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,

    pub added: Option<u64>,
    pub modified: Option<u64>,
    pub created: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<f32>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbhash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focal: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iso: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_space: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mp: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sspeed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub f_number: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acodecs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub achan: Option<Vec<usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcodecs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub long: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<MediaItemReference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<FileEpisode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub people: Option<Vec<MediaItemReference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbv: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbsize: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iv: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<RsLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub movie: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploader: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploadkey: Option<String>,
} 

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsGpsPosition {
    pub lat: f64,
    pub long: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaForUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub mimetype: Option<String>,
    pub size: Option<u64>,

    pub md5: Option<String>,
    
    pub modified: Option<u64>,
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

    pub duration: Option<u64>,
 
    pub progress: Option<usize>,

    pub add_tags: Option<Vec<MediaItemReference>>,
    pub remove_tags: Option<Vec<String>>,
    pub tags_lookup: Option<Vec<String>>,
    
    pub add_series: Option<Vec<FileEpisode>>,
    pub remove_series: Option<Vec<FileEpisode>>,

    pub add_people: Option<Vec<MediaItemReference>>,
    pub remove_people: Option<Vec<String>>,
    pub people_lookup: Option<Vec<String>>,

    pub long: Option<f64>,
    pub lat: Option<f64>,
    pub gps: Option<String>,

    pub origin: Option<RsLink>,
    pub origin_url: Option<String>,
    pub movie: Option<String>,

    pub lang: Option<String>,

    pub rating: Option<u16>,

    pub thumbsize: Option<usize>,
    pub iv: Option<String>,

    pub uploader: Option<String>,
    pub uploadkey: Option<String>,
    pub upload_id: Option<String>,

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
    pub lang: Option<String>,
    pub uploader: Option<String>,
    pub uploadkey: Option<String>,

    pub created: Option<i64>,
} 


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MediaForInsert {
    pub id: String,
    pub media: MediaForAdd
}

impl MediaForAdd {
    pub fn into_insert(self) -> MediaForInsert {
        MediaForInsert {id : nanoid!(), media: self}
    }
    pub fn into_insert_with_id(self, media_id: String) -> MediaForInsert {
        MediaForInsert {id : media_id, media: self}
    }
}



#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct GroupMediaDownload<T> {
    pub group: Option<bool>,
    pub group_thumbnail_url: Option<String>,
    pub group_filename: Option<String>,
    pub group_mime: Option<String>,
    pub files: Vec<T>,

    pub referer: Option<String>,
    pub cookies: Option<Vec<String>>,
    pub origin_url: Option<String>,

    pub title: Option<String>,

    pub people_lookup: Option<Vec<String>>,
    pub tags_lookup: Option<Vec<String>>,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MediaDownloadUrl {
    pub url: String,
    pub parse: bool,
    pub upload_id: Option<String>,
    //pub infos: Option<MediaForUpdate>,

    pub kind: Option<FileType>,
    pub filename: Option<String>,
    pub description: Option<String>,
    pub length: Option<u64>,
    pub thumbnail_url: Option<String>,
    
    pub people_lookup: Option<Vec<String>>,
    pub tags_lookup: Option<Vec<String>>,
}

impl From<MediaDownloadUrl> for RsRequest {
    fn from(value: MediaDownloadUrl) -> Self {
        RsRequest {
            url: value.url,
            mime: None,
            size: None,
            filename: value.filename,
            status: if value.parse { RsRequestStatus::NeedParsing } else { RsRequestStatus::Unprocessed },
            headers: None,
            cookies: None,
            files: None,
            selected_file: None,
            tags: value.tags_lookup,
            people: value.people_lookup,
            ..Default::default()
        }
    }
}

impl From<GroupMediaDownload<MediaDownloadUrl>> for Vec<RsRequest> {
    fn from(value: GroupMediaDownload<MediaDownloadUrl>) -> Self {
        let mut output = Vec::new();
        for file in value.files {
            output.push(
            RsRequest {
                        upload_id: file.upload_id,
                        url: file.url,
                        mime: None,
                        size: None,
                        filename: file.filename,
                        status: if file.parse { RsRequestStatus::NeedParsing } else { RsRequestStatus::Unprocessed },
                        headers: None,
                        cookies: value.cookies.as_ref().and_then(|c| c.iter().map(|s| RsCookie::from_str(s).ok()).collect()),
                        files: None,
                        selected_file: None,
                        referer: value.referer.clone(),
                        tags: file.tags_lookup.or(value.tags_lookup.clone()),
                        people: file.people_lookup.or(value.people_lookup.clone()),
                        description: file.description.or(value.title.clone()),
                        ..Default::default()
                    });
        }
        output
    }
}


impl From<Media> for MediaForAdd {
    fn from(value: Media) -> Self {
        MediaForAdd {
            name: value.name,
            description: value.description,
            people: value.people.map(|e| e.iter().map(|p| p.id.to_string()).collect::<Vec<String>>()),
            tags: value.tags.map(|e| e.iter().map(|p| p.id.to_string()).collect::<Vec<String>>()),
            long: value.long,
            lat: value.lat,
            created: value.created,
            origin: value.origin,
            series: value.series,
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
            ..Default::default()
        }
    }
}

impl From<RsRequest> for MediaForUpdate {
    fn from(value: RsRequest) -> Self {
        MediaForUpdate {
            name: value.filename,
            description: value.description,
            //kind: value.k,
            size: value.size,
            people_lookup: value.people,
            tags_lookup: value.tags,
            ..Default::default()
        }
    }
}

impl From<&SourceRead> for MediaForUpdate {
    fn from(value: &SourceRead) -> Self {
        match value {
            SourceRead::Stream(stream) => {
                MediaForUpdate {
                    name: stream.name.clone(),
                    mimetype: stream.mime.clone(),
                    size: stream.size.clone(),
                    ..Default::default()
                }
            },
            SourceRead::Request(r) => {
                MediaForUpdate {
                    name: r.filename.clone(),
                    mimetype: r.mime.clone(),
                    size: r.size.clone(),
                    ..Default::default()
                }
            },
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MediaWithAction {
    pub action: ElementAction,
    pub media: Media
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MediasMessage {
    pub library: String,
    pub medias: Vec<MediaWithAction>
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct ProgressMessage {
    pub library: String,
    pub progress: RsProgress
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct ConvertProgress {
    pub id: String,
    pub filename: String,
    pub converted_id: Option<String>,
    pub done: bool,
    pub percent: f64,
    pub request: Option<VideoConvertRequest>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct ConvertMessage {
    pub library: String,
    pub progress: ConvertProgress
}