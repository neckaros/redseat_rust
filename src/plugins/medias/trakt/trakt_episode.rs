use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::domain::episode::Episode;

use super::trakt_show::TraktIds;

#[derive(Debug, Serialize, Deserialize)]
pub struct TraktSeasonWithEpisodes {
    pub episodes: Vec<TraktFullEpisode>
}

/// An [episode] with full [extended info]
///
/// [episode]: https://trakt.docs.apiary.io/#reference/episodes
/// [extended info]: https://trakt.docs.apiary.io/#introduction/extended-info
#[derive(Debug, Serialize, Deserialize)]
pub struct TraktFullEpisode {
    pub season: u32,
    pub number: u32,
    pub title: Option<String>,
    pub ids: TraktIds,
    pub number_abs: Option<u32>,
    pub overview: Option<String>,
    pub rating: f32,
    pub votes: u32,
    pub comment_count: u32,
    pub first_aired: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub available_translations: Vec<String>,
    pub runtime: u32,
}

impl TraktFullEpisode {
    pub fn into_trakt(self, serie_ref: String) -> Episode {
        Episode {
            serie_ref,
            season: self.season,
            number: self.number,
            abs: self.number_abs,
            name: self.title,
            overview: self.overview,
            alt: None,
            airdate: self.first_aired.and_then(|t| Some(t.timestamp_millis() as u64)),
            duration: Some(self.runtime as u64),
            params: None,
            imdb: self.ids.imdb,
            slug: self.ids.slug,
            tmdb: self.ids.tmdb,
            trakt: self.ids.trakt,
            tvdb: self.ids.tvdb,
            otherids: None,
            imdb_rating: None,
            imdb_votes: None,
            trakt_rating: Some(self.rating),
            trakt_votes: Some(self.votes.into()),

            ..Default::default()     
        }
    }
}

