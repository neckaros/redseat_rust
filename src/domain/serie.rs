use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::{Display, EnumString};

pub use rs_plugin_common_interfaces::domain::serie::Serie;
pub use rs_plugin_common_interfaces::domain::serie::SerieStatus;

use crate::plugins::medias::imdb::ImdbContext;

use super::ElementAction;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SerieWithAction {
    pub action: ElementAction,
    pub serie: Serie,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SeriesMessage {
    pub library: String,
    pub series: Vec<SerieWithAction>,
}


#[async_trait::async_trait]
pub trait SerieExt {
    async fn fill_imdb_ratings(&mut self, imdb_context: &ImdbContext);
}

#[async_trait::async_trait]
impl SerieExt for Serie {
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