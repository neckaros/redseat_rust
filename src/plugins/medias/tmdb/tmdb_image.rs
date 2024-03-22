use serde::{Deserialize, Serialize};

use crate::model::series::ExternalSerieImages;

use super::tmdb_configuration::TmdbConfiguration;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmdbImage {
    #[serde(rename = "aspect_ratio")]
    pub aspect_ratio: f64,
    pub height: i64,
    #[serde(rename = "iso_639_1")]
    pub iso_639_1: String,
    #[serde(rename = "file_path")]
    pub file_path: String,
    #[serde(rename = "vote_average")]
    pub vote_average: f64,
    #[serde(rename = "vote_count")]
    pub vote_count: i64,
    pub width: i64,
}

pub trait ToBest {
    fn into_best(self) -> Option<TmdbImage>;
}

impl ToBest for Vec<TmdbImage> {
    fn into_best(mut self) -> Option<TmdbImage> {
        self.sort_by(|a, b| b.vote_average.partial_cmp(&a.vote_average).unwrap());
        self.into_iter().next()
    }
}

impl TmdbImage {
    pub fn full_path(&self, root: &str) -> String {
        format!("{}original{}", root, self.file_path)
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmdbImageResponse {
    pub backdrops: Vec<TmdbImage>,
    pub id: i64,
    pub logos: Vec<TmdbImage>,
    pub posters: Vec<TmdbImage>,
}

impl TmdbImageResponse {
    pub fn into_external(self, configuration: &TmdbConfiguration) -> ExternalSerieImages {
        ExternalSerieImages {
            backdrop: self.backdrops.into_best().and_then(|p| Some(p.full_path(&configuration.images.secure_base_url))),
            logo: self.logos.into_best().and_then(|p| Some(p.full_path(&configuration.images.secure_base_url))),
            poster:  self.posters.into_best().and_then(|p| Some(p.full_path(&configuration.images.secure_base_url))),
        }
    }
}