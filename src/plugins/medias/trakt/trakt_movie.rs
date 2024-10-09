use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use rs_plugin_common_interfaces::url::{RsLink, RsLinkType};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

use crate::domain::movie::{Movie, MovieStatus};

use super::trakt_show::TraktIds;


#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraktRelease {
    pub country: String,
    pub certification: Option<String>,
    #[serde(rename = "release_date")]
    pub release_date: NaiveDate,
    #[serde(rename = "release_type")]
    pub release_type: TraktReleaseType,
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
#[serde(rename_all = "lowercase")]
pub enum TraktReleaseType {
    Unknown,
    Premiere,
    Limited,
    Theatrical,
    Digital,
    Physical,
    Tv,
    #[serde(other)]
    #[default]
    Other,
}


#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone, Display, EnumString)]
#[serde(rename_all = "lowercase")]
pub enum TraktMovieStatus {
    Released,
    #[serde(rename = "in production")]
    InProduction,
    #[serde(rename = "post production")]
    PostProduction,
    Planned,
    Rumored,
    Canceled,
    #[serde(other)]
    #[default]
    Other,
}

impl From<TraktMovieStatus> for MovieStatus {
    fn from(value: TraktMovieStatus) -> Self {
        match value {
            TraktMovieStatus::Released => MovieStatus::Released,
            TraktMovieStatus::InProduction => MovieStatus::InProduction,
            TraktMovieStatus::PostProduction => MovieStatus::PostProduction,
            TraktMovieStatus::Planned => MovieStatus::Planned,
            TraktMovieStatus::Rumored => MovieStatus::Rumored,
            TraktMovieStatus::Canceled => MovieStatus::Canceled,
            TraktMovieStatus::Other => MovieStatus::Unknown,
        }
    }
}
//released, in production, post production, planned, rumored, or canceled.

pub trait TraktReleases {
    fn earliest_for(&self, kind: TraktReleaseType) -> Option<NaiveDate>;
}
impl TraktReleases for Vec<TraktRelease> {
    fn earliest_for(&self, kind: TraktReleaseType) -> Option<NaiveDate> {
        self.iter().filter(|r| &r.release_type == &kind).map(|r| r.release_date).min()
    }
}

/// A [movie] with full [extended info]
///
/// [movie]: https://trakt.docs.apiary.io/#reference/movies
/// [extended info]: https://trakt.docs.apiary.io/#introduction/extended-info
#[derive(Debug, Serialize, Deserialize)]
pub struct TraktFullMovie {
    pub title: String,
    pub year: Option<u16>,
    pub ids: TraktIds,
    pub tagline: String,
    pub overview: String,
    pub released: Option<NaiveDate>,
    pub runtime: Option<u32>,
    pub status: Option<TraktMovieStatus>,
    pub country: Option<String>,
    pub trailer: Option<String>,
    pub homepage: Option<String>,
    pub rating: f32,
    pub votes: u32,
    pub comment_count: u32,
    pub updated_at: Option<DateTime<Utc>>,
    pub language: Option<String>,
    pub available_translations: Vec<String>,
    pub genres: Vec<String>,
    pub certification: Option<String>,
}


impl From<TraktFullMovie> for Movie {
    fn from(value: TraktFullMovie) -> Self {
        Movie {
            id: format!("trakt:{}", value.ids.trakt.unwrap()),
            name: value.title,
            kind: None,
            year: value.year,
            airdate: None,
            digitalairdate: None,
            duration: value.runtime,
            overview: Some(value.overview),
            country: value.country,
            status: value.status.map(MovieStatus::from),
            imdb: value.ids.imdb,
            slug: value.ids.slug,
            tmdb: value.ids.tmdb,
            trakt: value.ids.trakt,
            otherids: None,
            lang: value.language,
            original: None,
            imdb_rating: None,
            imdb_votes: None,
            trakt_rating: Some(value.rating),
            trakt_votes: Some(value.votes),
            trailer: value.trailer.map(|t| RsLink {
                platform: "link".into(),
                kind: Some(RsLinkType::Post),
                id: t,
                ..Default::default()
            }),
            
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TraktTrendingMoviesResult {
    pub watchers: u64,
    pub movie: TraktFullMovie
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TraktMovieSearchElement {
    pub score: f64,
    pub movie: TraktFullMovie
}