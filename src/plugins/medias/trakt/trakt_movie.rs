use chrono::{DateTime, NaiveDate, Utc};
use rs_plugin_url_interfaces::{RsLink, RsLinkType};
use serde::{Deserialize, Serialize};

use crate::domain::movie::Movie;

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
    pub released: NaiveDate,
    pub runtime: Option<u32>,
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
            status: None,
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
            trailer: value.trailer.and_then(|t| Some(RsLink {
                platform: "link".into(),
                kind: Some(RsLinkType::Post),
                id: t,
                ..Default::default()
            })),
            
            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TraktTrendingMoviesResult {
    pub watchers: u64,
    pub movie: TraktFullMovie
}