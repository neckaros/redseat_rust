use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use strum_macros::{Display, EnumString};

use crate::domain::{serie::{Serie, SerieStatus}, MediasIds};

#[derive(Debug, Serialize, Deserialize, EnumString, Display, Default)]
#[serde(rename_all = "lowercase")]
pub enum TraktShowStatus {
    #[serde(rename = "returning series")]
    Returning,
    #[serde(rename = "in production")]
    InProduction,
    #[serde(rename = "post production")]
    PostProduction,
    Planned,
    Rumored,
    Cancelled,
    Ended,
    Released,
    Canceled,
    Pilot,
    #[strum(default)] Other(String),
    #[default] Unknown,
}

impl From<TraktShowStatus> for SerieStatus {
    fn from(value: TraktShowStatus) -> Self {
        match value {
            TraktShowStatus::Returning => SerieStatus::Returning,
            TraktShowStatus::InProduction => SerieStatus::InProduction,
            TraktShowStatus::PostProduction => SerieStatus::PostProduction,
            TraktShowStatus::Planned => SerieStatus::Planned,
            TraktShowStatus::Rumored => SerieStatus::Rumored,
            TraktShowStatus::Cancelled => SerieStatus::Canceled,
            TraktShowStatus::Ended => SerieStatus::Ended,
            TraktShowStatus::Released => SerieStatus::Released,
            TraktShowStatus::Canceled => SerieStatus::Canceled,
            TraktShowStatus::Pilot => SerieStatus::Pilot,
            TraktShowStatus::Other(s) => SerieStatus::Other(s),
            TraktShowStatus::Unknown => SerieStatus::Unknown,
        }
    }
}

/// Airing information of a [show]. Used in [FullShow]
///
/// [show]: https://trakt.docs.apiary.io/#reference/shows
/// [FullShow]: struct.FullShow.html
#[derive(Debug, Serialize, Deserialize)]
pub struct Airing {
    pub day: Option<String>,
    pub time: Option<String>,
    pub timezone: Option<String>,
}

/// [Ids] of almost every item
///
/// [Ids]: https://trakt.docs.apiary.io/#introduction/standard-media-objects
#[derive(Debug, Serialize, Deserialize, Ord, PartialOrd, Eq, PartialEq)]
pub struct TraktIds {
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

impl From<MediasIds> for TraktIds {
    fn from(value: MediasIds) -> Self {
        TraktIds { trakt: value.trakt, slug: value.slug, tvdb: value.tvdb, imdb: value.imdb, tmdb: value.tmdb, tvrage: value.tvrage }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TraktTrendingShowResult {
    pub watchers: u64,
    pub show: TraktFullShow
}

/// A [show] with full [extended info]
///
/// [show]: https://trakt.docs.apiary.io/#reference/shows
/// [extended info]: https://trakt.docs.apiary.io/#introduction/extended-info
#[derive(Debug, Serialize, Deserialize)]
pub struct TraktFullShow {
    pub title: String,
    pub year: Option<u16>,
    pub ids: TraktIds,
    pub overview: Option<String>,
    pub first_aired: Option<DateTime<Utc>>,
    pub airs: Airing,
    pub runtime: Option<u32>,
    pub certification: Option<String>,
    pub network: Option<String>,
    pub country: Option<String>,
    pub trailer: Option<String>,
    pub homepage: Option<String>,
    pub status: Option<TraktShowStatus>,
    pub rating: f64,
    pub votes: u32,
    pub comment_count: u32,
    pub updated_at: Option<DateTime<Utc>>,
    pub language: Option<String>,
    pub available_translations: Vec<String>,
    pub genres: Vec<String>,
    pub aired_episodes: u32,
}

impl From<TraktFullShow> for Serie {
    fn from(value: TraktFullShow) -> Self {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        Serie {
            id: format!("trakt:{}", value.ids.trakt.unwrap()),
            name: value.title,
            kind: None,
            status: value.status.map(SerieStatus::from),
            alt: None,
            params: None,
            imdb: value.ids.imdb,
            slug: value.ids.slug,
            tmdb: value.ids.tmdb,
            trakt: value.ids.trakt,
            tvdb: value.ids.tvdb,
            otherids: None,
            imdb_rating: None,
            imdb_votes: None,
            trakt_votes: Some(value.votes as u64),
            trakt_rating: Some(value.rating as f32),
            trailer: value.trailer,
            year: value.year,
            max_created: None,
            modified: t,
            added: t,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TraktShowSearchElement {
    pub score: f64,
    pub show: TraktFullShow
}


impl MediasIds {
    pub fn as_id_for_trakt(&self) -> Option<String> {
        if let Some(trakt) = self.trakt {
            Some(trakt.to_string())
        } else { self.imdb.as_ref().map(|imdb| imdb.to_string()) }
    }
}

