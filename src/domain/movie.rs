use rs_plugin_common_interfaces::url::RsLink;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::{Display, EnumString};

use crate::tools::serialization_tools::rating_serializer;

use super::ElementAction;

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone, Display, EnumString)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum MovieStatus {
    Returning,
    Released,
    InProduction,
    PostProduction,
    Planned,
    Rumored,
    Canceled,
    #[strum(default)] Other(String),
    #[default] Unknown,

}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Movie {

    #[serde(default)]
    pub id: String,
    
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Option<Value>,
    pub year: Option<u16>,
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

    #[serde(serialize_with = "rating_serializer")]
    pub imdb_rating: Option<f32>,
    pub imdb_votes: Option<u64>,
    #[serde(serialize_with = "rating_serializer")]
    pub trakt_rating: Option<f32>,
    pub trakt_votes: Option<u32>,

    pub trailer: Option<RsLink>,
    
    #[serde(default)]
    pub modified: u64,
    #[serde(default)]
    pub added: u64,


    #[serde(skip_serializing_if = "Option::is_none")]
    pub watched: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u64>,

    #[serde(default)]
    pub posterv: u64,
    #[serde(default)]
    pub backgroundv: u64,
    #[serde(default)]
    pub cardv: u64

}


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