use std::str::FromStr;

use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;

use super::{rs_link::RsLink, ElementAction};
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
   id: String,
   conf: u16
}

impl FromStr for MediaTagReference {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let splitted: Vec<&str> = s.split("|").collect();
        if splitted.len() == 2 {
            Ok(MediaTagReference { id: splitted[0].to_string(), conf: splitted[1].parse::<u16>().unwrap_or(101) })
        } else {
            Ok(MediaTagReference { id: splitted[0].to_string(), conf: 101 })
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone, strum_macros::Display,EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum FileType {
    Directory,
    Photo,
    Video,
    Archive,
    Album,
    Other
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub id: String,
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
    pub created: Option<u64>,

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MediaForUpdate {
    name: Option<String>,
    description: Option<String>,
    mimetype: Option<String>,
    size: Option<usize>,
    

    modified: Option<u64>,
    created: Option<u64>,

    width: Option<usize>,
    height: Option<usize>,
  
    duration: Option<usize>,
 
    progress: Option<usize>,

    add_tags: Option<Vec<String>>,
    remove_tags: Option<Vec<String>>,

    add_series: Option<Vec<FileEpisode>>,
    remove_series: Option<Vec<FileEpisode>>,

    add_people: Option<Vec<String>>,
    remove_people: Option<Vec<String>>,

    long: Option<usize>,
    lat: Option<usize>,

    origin: Option<RsLink>,
    movie: Option<String>,

    lang: Option<String>,

    uploader: Option<String>,
    uploadkey: Option<String>,
} 


#[derive(Debug, Serialize, Deserialize, Clone)]
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
pub struct GroupMediaDownload {
    pub group: Option<bool>,
    pub group_thumbnail_url: Option<String>,
    pub group_filename: Option<String>,
    pub group_mime: Option<String>,
    pub files: Option<Vec<MediaForUpdate>>,

    pub title: Option<String>,
}

pub struct MediaDownloadUrl {
    pub url: String,
    pub infos: Option<MediaForUpdate>
}




#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MediasMessage {
    pub library: String,
    pub action: ElementAction,
    pub medias: Vec<Media>
}