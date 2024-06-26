use std::default;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use socketioxide::extract::SocketRef;
use strum_macros::EnumString;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct RsPlayerAvailable {
    pub socket: SocketRef,
    pub uid: String,
	pub player: RsPlayer,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, PartialOrd, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsPlayer {
    pub name: String,
	pub player: String,
}



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, PartialOrd, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsPlayerEvent {
    pub id: String,
    pub name: String,
	pub player: String,
}

impl From<RsPlayerAvailable> for RsPlayerEvent {
    fn from(value: RsPlayerAvailable) -> Self {
        Self {
            id: value.socket.id.to_string(),
            name: value.player.name,
            player: value.player.player,
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, PartialOrd, Default)]
#[serde(rename_all = "camelCase")]
pub struct RsPlayerPlayRequest {
    pub id: String,
    pub url: String,
    #[serde(rename = "type")]
	pub kind: Option<String>,
	pub library_id: Option<String>,
	pub movie: Option<String>,
	pub show: Option<String>,
	pub episode: Option<u32>,
	pub season: Option<u32>,
}




#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display,EnumString)]
#[strum(serialize_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub enum RsPlayerAction {
    Stop,
    Pause,
    Play,
    Playpause,
    Forward,
    Backward
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RsPlayerActionRequest {
    pub action: RsPlayerActionDetail,
    pub id: String,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RsPlayerActionDetail {
    pub action: RsPlayerAction,
    pub options: Option<Value>,
}
