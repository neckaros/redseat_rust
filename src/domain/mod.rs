use serde::{Deserialize, Serialize};

use self::serie::Serie;

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
    pub redseat: Option<String>,
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

impl MediasIds {
    pub fn into_best(self) -> Option<String> {
        self.redseat.or(self.trakt.and_then(|r| Some(r.to_string()))).or(self.imdb)
    }

    pub fn from_imdb(imdb: String) -> Self {
        MediasIds {
            imdb: Some(imdb),
            ..Default::default()
        }
    }
    pub fn from_trakt(trakt: u64) -> Self {
        MediasIds {
            trakt: Some(trakt),
            ..Default::default()
        }
    }
    pub fn from_tvdb(tvdb: u64) -> Self {
        MediasIds {
            tvdb: Some(tvdb),
            ..Default::default()
        }
    }
    pub fn from_tmdb(tmdb: u64) -> Self {
        MediasIds {
            tmdb: Some(tmdb),
            ..Default::default()
        }
    }

    pub fn as_id(&self) -> crate::Result<String> {
        if let Some(imdb) = &self.imdb {
            Ok(format!("imdb:{}", imdb))
        } else if let Some(trakt) = &self.trakt {
            Ok(format!("trakt:{}", trakt))
        } else if let Some(tmdb) = &self.tmdb {
            Ok(format!("tmdb:{}", tmdb))
        } else if let Some(tvdb) = &self.tvdb {
            Ok(format!("tmdb:{}", tvdb))
        } else {
            Err(crate::Error::NoMediaIdRequired(self.clone()))
        }
    }   

    pub fn is_id(id: &str) -> bool {
        id.contains(":") && id.split(":").count() == 2
    }
}

impl From<Serie> for MediasIds {
    fn from(value: Serie) -> Self {
        MediasIds { redseat: Some(value.id), trakt: value.trakt, slug: value.slug, tvdb: value.tvdb, imdb: value.imdb, tmdb: value.tmdb, tvrage: None }
    }
}

impl TryFrom<String> for MediasIds {
    type Error = crate::Error;
    fn try_from(value: String) -> crate::Result<MediasIds> {
        if !Self::is_id(&value) {
            return Err(crate::Error::NotAMediaId(value))
        }
        let elements = value.split(":").collect::<Vec<_>>();
        let source = elements.get(0).unwrap();
        let id = elements.get(1).unwrap();

        if *source == "imdb" {
            Ok(MediasIds::from_imdb(id.to_string()))
        } else if *source == "trakt" {
            let id: u64 = id.parse().map_err(|_| crate::Error::NotAMediaId(value))?;
            Ok(MediasIds::from_trakt(id))
        } else if *source == "tmdb" {
            let id: u64 = id.parse().map_err(|_| crate::Error::NotAMediaId(value))?;
            Ok(MediasIds::from_tmdb(id))
        } else if *source == "tvdb" {
            let id: u64 = id.parse().map_err(|_| crate::Error::NotAMediaId(value))?;
            Ok(MediasIds::from_tvdb(id))
        } else{
            Err(crate::Error::NotAMediaId(value))
        }
        
    }
    
    
}