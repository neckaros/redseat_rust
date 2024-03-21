use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::trakt_show::TraktIds;

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