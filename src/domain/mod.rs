use serde::{Deserialize, Serialize};

pub mod file;
pub mod library;
pub mod ffmpeg;
pub mod credential;
pub mod backup;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub enum ElementAction {
    Removed,
    Added,
    Updated
}