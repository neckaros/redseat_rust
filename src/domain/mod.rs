use serde::{Deserialize, Serialize};

pub mod media;
pub mod library;
pub mod ffmpeg;
pub mod credential;
pub mod backup;
pub mod tag;
pub mod rs_link;
pub mod people;
pub mod serie;
pub mod episode;
pub mod plugin;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub enum ElementAction {
    Removed,
    Added,
    Updated
}

#[derive(Debug, Clone, Serialize, Deserialize, Ord, PartialOrd, Eq, PartialEq, Default)]
pub struct MediasIds {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trakt: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tvdb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imdb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tmdb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tvrage: Option<u64>,
}