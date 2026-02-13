use people::Person;
use rs_plugin_common_interfaces::domain::rs_ids::RsIds;
use serde::{Deserialize, Serialize};

use crate::error::RsResult;


pub use rs_plugin_common_interfaces::domain::MediaElement;

use rs_plugin_common_interfaces::domain::book::Book;
use self::{episode::Episode, media::Media, movie::Movie, serie::Serie};

/// Extension trait for RsIds to get all possible external IDs
pub trait RsIdsExt {
    /// Returns all non-None external IDs as formatted strings
    fn into_all_external(self) -> Vec<String>;
    /// Returns all non-None external IDs plus local redseat ID
    fn into_all_external_or_local(self) -> Vec<String>;
}

impl RsIdsExt for RsIds {
    fn into_all_external(self) -> Vec<String> {
        let mut ids = Vec::new();
        if let Some(imdb) = self.imdb {
            ids.push(format!("imdb:{}", imdb));
        }
        if let Some(trakt) = self.trakt {
            ids.push(format!("trakt:{}", trakt));
        }
        if let Some(tmdb) = self.tmdb {
            ids.push(format!("tmdb:{}", tmdb));
        }
        if let Some(tvdb) = self.tvdb {
            ids.push(format!("tvdb:{}", tvdb));
        }
        if let Some(slug) = self.slug {
            ids.push(format!("slug:{}", slug));
        }
        ids
    }

    fn into_all_external_or_local(self) -> Vec<String> {
        let redseat = self.redseat.clone();
        let mut ids = self.into_all_external();
        if let Some(redseat) = redseat {
            ids.push(format!("redseat:{}", redseat));
        }
        ids
    }
}

pub mod backup;
pub mod book;
pub mod credential;
pub mod deleted;
pub mod episode;
pub mod ffmpeg;
pub mod library;
pub mod media;
pub mod media_progress;
pub mod media_rating;
pub mod movie;
pub mod people;
pub mod plugin;
pub mod request_processing;
pub mod rs_link;
pub mod serie;
pub mod tag;
pub mod view_progress;
pub mod watched;

pub mod progress;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum ElementAction {
    Deleted,
    Added,
    Updated,
}