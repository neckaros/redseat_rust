use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ElementAction;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")] 
pub struct Episode {
    pub serie_ref: String,
    pub season: usize,
    pub number: usize,

    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abs: Option<usize>,

	pub name: Option<String>,
	pub overview: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<Vec<String>>,

    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub airdate: Option<u64>,
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
    
    pub imdb_rating: Option<f32>,
    pub imdb_votes: Option<u64>,
    pub trakt_rating: Option<f32>,
    pub trakt_votes: Option<u64>,
       

    pub modified: u64,
    pub added: u64
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct EpisodesMessage {
    pub library: String,
    pub action: ElementAction,
    pub episodes: Vec<Episode>
}