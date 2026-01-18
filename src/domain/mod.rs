use people::Person;
use rs_plugin_common_interfaces::domain::rs_ids::RsIds;
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
pub mod media_progress;
pub mod media_rating;
pub mod request_processing;

pub mod progress;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub enum ElementAction {
    Deleted,
    Added,
    Updated
}


impl From<Serie> for RsIds {
    fn from(value: Serie) -> Self {
        RsIds { redseat: Some(value.id), trakt: value.trakt, slug: value.slug, tvdb: value.tvdb, imdb: value.imdb, tmdb: value.tmdb, tvrage: None, other_ids: None }
    }
}
impl From<Episode> for RsIds {
    fn from(value: Episode) -> Self {
        RsIds { redseat: Some(value.id()), trakt: value.trakt, slug: value.slug, tvdb: value.tvdb, imdb: value.imdb, tmdb: value.tmdb, tvrage: None, other_ids: None }
    }
}


impl From<Movie> for RsIds {
    fn from(value: Movie) -> Self {
        RsIds { redseat: Some(value.id), trakt: value.trakt, slug: value.slug, tvdb: None, imdb: value.imdb, tmdb: value.tmdb, tvrage: None, other_ids: None }
    }
}
impl From<Person> for RsIds {
    fn from(value: Person) -> Self {
        RsIds { redseat: Some(value.id), trakt: value.trakt, slug: value.slug, tvdb: None, imdb: value.imdb, tmdb: value.tmdb, tvrage: None, other_ids: None }
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
