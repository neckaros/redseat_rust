use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tools::serialization_tools::rating_serializer;

use super::ElementAction;


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct Episode {
    pub serie: String,
    pub season: u32,
    pub number: u32,

    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abs: Option<u32>,

	pub name: Option<String>,
	pub overview: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<Vec<String>>,

    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub airdate: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
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


    #[serde(skip_serializing_if = "Option::is_none")]
    pub watched: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u64>,
       
    #[serde(default)]
    pub modified: u64,
    #[serde(default)]
    pub added: u64,
    
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serie_name: Option<String>
}

impl Episode {
    pub fn id(&self) -> String {
        format!("{}x{}x{}", self.serie, self.season, self.number)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodeWithShow {
    pub name: String,
    pub episode: Episode
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct EpisodeWithAction {
    pub action: ElementAction,
    pub episode: Episode
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct EpisodesMessage {
    pub library: String,
    pub episodes: Vec<EpisodeWithAction>
}