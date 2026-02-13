use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{plugins::medias::imdb::ImdbContext, tools::serialization_tools::rating_serializer};
pub use rs_plugin_common_interfaces::domain::episode::Episode;
use super::ElementAction;


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

#[async_trait::async_trait]
pub trait EpisodeExt {
    async fn fill_imdb_ratings(&mut self, imdb_context: &ImdbContext);
}

#[async_trait::async_trait]
impl EpisodeExt for Episode {
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