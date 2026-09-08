use rs_plugin_common_interfaces::url::RsLink;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::{Display, EnumString};

use crate::{plugins::medias::imdb::ImdbContext, tools::serialization_tools::rating_serializer};

use super::ElementAction;
pub use rs_plugin_common_interfaces::domain::movie::{Movie, MovieStatus};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieForUpdate {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<Value>,
    pub year: Option<u32>,
    pub airdate: Option<i64>,
    pub digitalairdate: Option<i64>,

    pub duration: Option<u32>,
    pub overview: Option<String>,
    pub country: Option<String>,
    pub status: Option<MovieStatus>,

    pub imdb: Option<String>,
    pub slug: Option<String>,
    pub tmdb: Option<u64>,
    pub trakt: Option<u64>,
    pub otherids: Option<String>,

    pub lang: Option<String>,
    pub original: Option<String>,
    #[serde(rename = "imdb_rating")]
    pub imdb_rating: Option<f32>,
    #[serde(rename = "imdb_votes")]
    pub imdb_votes: Option<u64>,
    #[serde(rename = "trakt_rating")]
    pub trakt_rating: Option<f32>,
    #[serde(rename = "trakt_votes")]
    pub trakt_votes: Option<u32>,
    pub trailer: Option<RsLink>,
}

impl MovieForUpdate {
    pub fn from_refresh(movie: &Movie, new_movie: Movie) -> serde_json::Result<Self> {
        let mut updates = MovieForUpdate {
            ..Default::default()
        };

        if movie.name != new_movie.name {
            updates.name = Some(new_movie.name);
        }
        if movie.kind != new_movie.kind {
            updates.kind = new_movie.kind;
        }
        if movie.year != new_movie.year {
            updates.year = new_movie.year.map(u32::from);
        }
        if movie.duration != new_movie.duration {
            updates.duration = new_movie.duration;
        }
        if movie.overview != new_movie.overview {
            updates.overview = new_movie.overview;
        }
        if movie.country != new_movie.country {
            updates.country = new_movie.country;
        }
        if movie.lang != new_movie.lang {
            updates.lang = new_movie.lang;
        }
        if movie.original != new_movie.original {
            updates.original = new_movie.original;
        }
        if movie.slug != new_movie.slug {
            updates.slug = new_movie.slug;
        }
        if movie.trakt != new_movie.trakt {
            updates.trakt = new_movie.trakt;
        }
        if movie.otherids != new_movie.otherids {
            updates.otherids = new_movie
                .otherids
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
        }
        if movie.imdb_rating != new_movie.imdb_rating {
            updates.imdb_rating = new_movie.imdb_rating;
        }
        if movie.imdb_votes != new_movie.imdb_votes {
            updates.imdb_votes = new_movie.imdb_votes;
        }
        if movie.status != new_movie.status {
            updates.status = new_movie.status;
        }
        if movie.trakt_rating != new_movie.trakt_rating {
            updates.trakt_rating = new_movie.trakt_rating;
        }
        if movie.trakt_votes != new_movie.trakt_votes {
            updates.trakt_votes = new_movie.trakt_votes;
        }
        if movie.trailer != new_movie.trailer {
            updates.trailer = new_movie.trailer;
        }
        if movie.imdb != new_movie.imdb {
            updates.imdb = new_movie.imdb;
        }
        if movie.tmdb != new_movie.tmdb {
            updates.tmdb = new_movie.tmdb;
        }
        if movie.digitalairdate != new_movie.digitalairdate {
            updates.digitalairdate = new_movie.digitalairdate;
        }
        if movie.airdate != new_movie.airdate {
            updates.airdate = new_movie.airdate;
        }
        Ok(updates)
    }

    pub fn has_update(&self) -> bool {
        self.name.is_some()
            || self.kind.is_some()
            || self.status.is_some()
            || self.digitalairdate.is_some()
            || self.airdate.is_some()
            || self.imdb.is_some()
            || self.slug.is_some()
            || self.tmdb.is_some()
            || self.trakt.is_some()
            || self.otherids.is_some()
            || self.imdb_rating.is_some()
            || self.imdb_votes.is_some()
            || self.trakt_rating.is_some()
            || self.trakt_votes.is_some()
            || self.trailer.is_some()
            || self.year.is_some()
            || self.duration.is_some()
            || self.overview.is_some()
            || self.country.is_some()
            || self.lang.is_some()
            || self.original.is_some()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MovieWithAction {
    pub action: ElementAction,
    pub movie: Movie,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MoviesMessage {
    pub library: String,
    pub movies: Vec<MovieWithAction>,
}

#[async_trait::async_trait]
pub trait MovieExt {
    async fn fill_imdb_ratings(&mut self, imdb_context: &ImdbContext);
}

#[async_trait::async_trait]
impl MovieExt for Movie {
    async fn fill_imdb_ratings(&mut self, imdb_context: &ImdbContext) {
        if let Some(imdb) = &self.imdb {
            let rating = imdb_context.get_rating(imdb).await.unwrap_or(None);
            if let Some(rating) = rating {
                self.imdb_rating = Some(rating.0);
                self.imdb_votes = Some(rating.1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Movie, MovieForUpdate};
    use rs_plugin_common_interfaces::domain::other_ids::OtherIds;

    #[test]
    fn movie_refresh_copies_changed_other_ids_and_preserves_missing_ids() {
        let mut movie = Movie::default();
        for value in ["provider:first", "provider:changed"] {
            let ids = OtherIds::from(vec![value.to_string()]);
            let refreshed = Movie {
                otherids: Some(ids.clone()),
                ..movie.clone()
            };
            let update = MovieForUpdate::from_refresh(&movie, refreshed.clone()).unwrap();
            assert!(update.has_update());
            assert_eq!(
                serde_json::from_str::<OtherIds>(update.otherids.as_ref().unwrap()).unwrap(),
                ids
            );
            movie = refreshed;
        }
        let unchanged = MovieForUpdate::from_refresh(&movie, movie.clone()).unwrap();
        assert!(!unchanged.has_update());
        let missing = MovieForUpdate::from_refresh(&movie, Movie::default()).unwrap();
        assert!(missing.otherids.is_none());
        assert!(!missing.has_update());
    }

    #[test]
    fn movie_update_duration_rejects_values_outside_stored_range() {
        for duration in [0, u32::MAX as u64] {
            let update: MovieForUpdate =
                serde_json::from_value(serde_json::json!({"duration": duration})).unwrap();
            assert_eq!(update.duration, Some(duration as u32));
        }
        for duration in [u32::MAX as u64 + 1, i64::MAX as u64] {
            assert!(serde_json::from_value::<MovieForUpdate>(
                serde_json::json!({"duration": duration})
            )
            .is_err());
        }
    }
}
