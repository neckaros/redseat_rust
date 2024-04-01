use std::str::FromStr;

use nanoid::nanoid;
use plugin_request_interfaces::{RsRequest, RsRequestStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;

use rs_plugin_url_interfaces::RsLink;
use super::{progress::RsProgress, ElementAction};
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileEpisode {
   id: String,
   season: Option<usize>,
   episode: Option<usize>
}

impl FromStr for FileEpisode {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let splitted: Vec<&str> = s.split("|").collect();
        if splitted.len() == 3 {
            Ok(FileEpisode { id: splitted[0].to_string(), season: splitted[1].parse::<usize>().ok(), episode: splitted[2].parse::<usize>().ok() })
        } else if splitted.len() == 2 {
            Ok(FileEpisode { id: splitted[0].to_string(), season: splitted[1].parse::<usize>().ok(), episode: None })
        } else {
            Ok(FileEpisode { id: splitted[0].to_string(), season: None, episode: None })
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MediaTagReference {
   pub id: String,
   #[serde(skip_serializing_if = "Option::is_none")]
   pub conf: Option<u16>
}

impl FromStr for MediaTagReference {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let splitted: Vec<&str> = s.split("|").collect();
        if splitted.len() == 2 {
            Ok(MediaTagReference { id: splitted[0].to_string(), conf: splitted[1].parse::<u16>().ok().and_then(|e| if e == 100 {None} else {Some(e)}) })
        } else {
            Ok(MediaTagReference { id: splitted[0].to_string(), conf: None })
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
    pub mimetype: Option<String>,
    pub size: Option<usize>,
    
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
    pub focal: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iso: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_space: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sspeed: Option<String>,
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
    pub fps: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<usize>,

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
    pub tags: Option<Vec<MediaTagReference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<FileEpisode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub people: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbv: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbsize: Option<usize>,
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
pub struct MediaForUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub mimetype: Option<String>,
    pub size: Option<u64>,

    pub md5: Option<String>,
    
    pub modified: Option<u64>,
    pub created: Option<u64>,

    pub width: Option<usize>,
    pub height: Option<usize>,
  
    pub duration: Option<usize>,
 
    pub progress: Option<usize>,

    pub add_tags: Option<Vec<MediaTagReference>>,
    pub remove_tags: Option<Vec<String>>,

    pub add_series: Option<Vec<FileEpisode>>,
    pub remove_series: Option<Vec<FileEpisode>>,

    pub add_people: Option<Vec<String>>,
    pub remove_people: Option<Vec<String>>,
    pub people_lookup: Option<Vec<String>>,

    pub long: Option<usize>,
    pub lat: Option<usize>,

    pub origin: Option<RsLink>,
    pub movie: Option<String>,

    pub lang: Option<String>,

    pub uploader: Option<String>,
    pub uploadkey: Option<String>,

} 


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct MediaForAdd {
    pub source: Option<String>,
    pub name: String,
    pub description: Option<String>,

    #[serde(rename = "type")]
    pub kind: FileType,
    pub mimetype: Option<String>,
    pub size: Option<usize>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,

    pub rating: Option<u8>,
    pub md5: Option<String>,

    pub width: Option<usize>,
    pub height: Option<usize>,
    pub phash: Option<String>,
    pub thumbhash: Option<String>,
    pub focal: Option<usize>,
    pub iso: Option<usize>,
    pub color_space: Option<String>,
    pub sspeed: Option<String>,
    pub orientation: Option<usize>,

    pub duration: Option<usize>,
    pub acodecs: Option<Vec<String>>,
    pub achan: Option<Vec<usize>>,
    pub vcodecs: Option<Vec<String>>,
    pub fps: Option<f32>,
    pub bitrate: Option<usize>,

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

    pub created: Option<u64>,
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

pub trait GroupMediaDownloadContent {
    fn infos(&self) -> Option<MediaForUpdate>;
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct GroupMediaDownload<T>
where T: GroupMediaDownloadContent {
    pub group: Option<bool>,
    pub group_thumbnail_url: Option<String>,
    pub group_filename: Option<String>,
    pub group_mime: Option<String>,
    pub files: Vec<T>,

    pub referer: Option<String>,
    pub cookies: Option<Vec<String>>,

    pub title: Option<String>,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MediaDownloadUrl {
    pub url: String,
    pub parse: bool,
    pub upload_id: Option<String>,
    pub infos: Option<MediaForUpdate>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MediaDownloadUrlWithId {
    pub url: String,
    pub parse: bool,
    pub upload_id: String,
    pub infos: Option<MediaForUpdate>
}

impl From<MediaDownloadUrl> for MediaDownloadUrlWithId {
    fn from(value: MediaDownloadUrl) -> Self {
        Self {
            url: value.url,
            parse: value.parse,
            upload_id: value.upload_id.unwrap_or_else(|| nanoid!()),
            infos: value.infos,
        }
    }
}

impl From<MediaDownloadUrl> for RsRequest {
    fn from(value: MediaDownloadUrl) -> Self {
        RsRequest {
            url: value.url,
            mime: (value.infos.clone()).and_then(|i| i.mimetype.clone()),
            size: None,
            filename: value.infos.and_then(|i| i.name.clone()),
            status: if value.parse { RsRequestStatus::NeedParsing } else { RsRequestStatus::Unprocessed },
            headers: None,
            cookies: None,
            files: None,
            selected_file: None,
        }
    }
}

impl GroupMediaDownloadContent for MediaDownloadUrl {
    fn infos(&self) -> Option<MediaForUpdate> {
        self.infos.clone()
    }
}



#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MediasMessage {
    pub library: String,
    pub action: ElementAction,
    pub medias: Vec<Media>
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct ProgressMessage {
    pub library: String,
    pub name: String,
    pub progress: RsProgress
}