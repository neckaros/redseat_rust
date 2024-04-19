use serde::{Deserialize, Serialize};

use crate::error::RsResult;

use self::{episode::Episode, media::Media, movie::Movie, serie::Serie};

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
pub mod movie;
pub mod watched;
pub mod deleted;
pub mod view_progress;

pub mod progress;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub enum ElementAction {
    Deleted,
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
    pub fn try_add(&mut self, value: String) -> RsResult<()> {
        if !Self::is_id(&value) {
            return Err(crate::Error::NotAMediaId(value))
        }
        let elements = value.split(":").collect::<Vec<_>>();
        let source = elements.get(0).unwrap();
        let id = elements.get(1).unwrap();

        if *source == "redseat" {
            self.redseat = Some(id.to_string());
            Ok(())
        } else if *source == "imdb" {
            self.imdb = Some(id.to_string());
            Ok(())
        } else if *source == "trakt" {
            let id: u64 = id.parse().map_err(|_| crate::Error::NotAMediaId(value))?;
            self.trakt = Some(id);
            Ok(())
        } else if *source == "tmdb" {
            let id: u64 = id.parse().map_err(|_| crate::Error::NotAMediaId(value))?;
            self.tmdb = Some(id);
            Ok(())
        } else if *source == "tvdb" {
            let id: u64 = id.parse().map_err(|_| crate::Error::NotAMediaId(value))?;
            self.tvdb = Some(id);
            Ok(())
        } else if *source == "tvrage" {
            let id: u64 = id.parse().map_err(|_| crate::Error::NotAMediaId(value))?;
            self.tvrage = Some(id);
            Ok(())
        } else{
            Err(crate::Error::NotAMediaId(value))
        }  
    }

    pub fn into_best(self) -> Option<String> {
        self.as_redseat().or(self.into_best_external())
    }

    pub fn into_best_external(self) -> Option<String> {
        self.as_trakt().or(self.as_imdb()).or(self.as_tmdb()).or(self.as_tvdb())
    }

    pub fn from_imdb(imdb: String) -> Self {
        MediasIds {
            imdb: Some(imdb),
            ..Default::default()
        }
    }
    pub fn as_imdb(&self) -> Option<String> {
        self.imdb.as_ref().map(|i| format!("imdb:{}", i))
    }
    
    pub fn from_trakt(trakt: u64) -> Self {
        MediasIds {
            trakt: Some(trakt),
            ..Default::default()
        }
    }
    pub fn as_trakt(&self) -> Option<String> {
        self.trakt.map(|i| format!("trakt:{}", i))
    }

    pub fn from_tvdb(tvdb: u64) -> Self {
        MediasIds {
            tvdb: Some(tvdb),
            ..Default::default()
        }
    }
    pub fn as_tvdb(&self) -> Option<String> {
        self.tvdb.map(|i| format!("tvdb:{}", i))
    }

    pub fn from_tmdb(tmdb: u64) -> Self {
        MediasIds {
            tmdb: Some(tmdb),
            ..Default::default()
        }
    }
    pub fn as_tmdb(&self) -> Option<String> {
        self.tmdb.map(|i| format!("tmdb:{}", i))
    }

    pub fn from_redseat(redseat: String) -> Self {
        MediasIds {
            redseat: Some(redseat),
            ..Default::default()
        }
    }
    pub fn as_redseat(&self) -> Option<String> {
        self.redseat.as_ref().map(|i| format!("redseat:{}", i))
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
            Err(crate::Error::NoMediaIdRequired(Box::new(self.clone())))
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
impl From<Episode> for MediasIds {
    fn from(value: Episode) -> Self {
        MediasIds { redseat: Some(value.id()), trakt: value.trakt, slug: value.slug, tvdb: value.tvdb, imdb: value.imdb, tmdb: value.tmdb, tvrage: None }
    }
}


impl From<Movie> for MediasIds {
    fn from(value: Movie) -> Self {
        MediasIds { redseat: Some(value.id), trakt: value.trakt, slug: value.slug, tvdb: None, imdb: value.imdb, tmdb: value.tmdb, tvrage: None }
    }
}

impl TryFrom<String> for MediasIds {
    type Error = crate::Error;
    fn try_from(value: String) -> crate::Result<MediasIds> {
        let mut id = MediasIds::default();
        id.try_add(value)?;
        Ok(id)
 
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub enum MediaElement {
	Media(Media),
    Movie(Movie),
    Episode(Episode),
    Serie(Serie)
}
