use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::{Display, EnumString};

use crate::{model::series::SerieForUpdate, tools::serialization_tools::rating_serializer};

use super::ElementAction;

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone, Display, EnumString)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum SerieStatus {
    Returning,
    InProduction,
    PostProduction,
    Planned,
    Rumored,
    Ended,
    Released,
    Canceled,
    Pilot,
    #[strum(default)] Other(String),
    #[default] Unknown,
}

#[derive(Debug, Serialize, PartialEq, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Serie {

    #[serde(default)]
    pub id: String,
    
	pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<SerieStatus>,
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imdb: Option<String>,
    pub slug: Option<String>,
    pub tmdb: Option<u64>,
    pub trakt: Option<u64>,
    pub tvdb: Option<u64>,
    pub otherids: Option<String>,
    
    #[serde(serialize_with = "rating_serializer")]
    pub imdb_rating: Option<f32>,
    pub imdb_votes: Option<u64>,
    #[serde(serialize_with = "rating_serializer")]
    pub trakt_rating: Option<f32>,
    pub trakt_votes: Option<u64>,

    pub trailer: Option<String>,


    pub year: Option<u16>,

    
    pub max_created: Option<u64>,


    #[serde(default)]
    pub modified: u64,
    #[serde(default)]
    pub added: u64,

    
    #[serde(default)]
    pub posterv: u64,
    #[serde(default)]
    pub backgroundv: u64,
    #[serde(default)]
    pub cardv: u64
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct SerieWithAction {
    pub action: ElementAction,
    pub serie: Serie
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct SeriesMessage {
    pub library: String,
    pub series: Vec<SerieWithAction>
}