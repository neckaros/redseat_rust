use serde::{Deserialize, Serialize};
use serde_json::Value;


use super::ElementAction;
use rs_plugin_common_interfaces::{url::RsLink, Gender};


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
	pub name: String,
    pub socials: Option<Vec<RsLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portrait: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birthday: Option<i64>,
    pub modified: u64,
    pub added: u64,
    pub posterv: u32,
    #[serde(default)]
    pub generated: bool,


    pub imdb: Option<String>,
    pub slug: Option<String>,
    pub tmdb: Option<u64>,
    pub trakt: Option<u64>,

        
    pub death: Option<i64>,
    pub gender: Option<Gender>,
    pub country: Option<String>,
    pub bio: Option<String>,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct PeopleMessage {
    pub library: String,
    pub people: Vec<PersonWithAction>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct PersonWithAction {
    pub action: ElementAction,
    pub person: Person
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct FaceBBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct UnassignedFace {
    pub id: String,
    pub embedding: Vec<f32>,
    pub media_ref: String,
    pub bbox: FaceBBox,
    pub confidence: f32,
    pub pose: Option<(f32, f32, f32)>,
    pub cluster_id: Option<String>,
    pub created: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FaceEmbedding {
    pub id: String,
    pub embedding: Vec<f32>,
    pub media_ref: Option<String>,
    pub bbox: Option<FaceBBox>,
    pub confidence: Option<f32>,
    pub pose: Option<(f32, f32, f32)>,
}