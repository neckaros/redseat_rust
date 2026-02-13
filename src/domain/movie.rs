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

    pub duration: Option<u64>,
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
    pub fn has_update(&self) -> bool {
        self.name.is_some() || self.kind.is_some() || self.status.is_some()
        || self.digitalairdate.is_some() || self.airdate.is_some() 
        || self.imdb.is_some() || self.slug.is_some() || self.tmdb.is_some() || self.trakt.is_some() || self.otherids.is_some()
        || self.imdb_rating.is_some() || self.imdb_votes.is_some() || self.trakt_rating.is_some() || self.trakt_votes.is_some()
        || self.trailer.is_some() || self.year.is_some()
    } 
}



#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MovieWithAction {
    pub action: ElementAction,
    pub movie: Movie
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MoviesMessage {
    pub library: String,
    pub movies: Vec<MovieWithAction>
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

