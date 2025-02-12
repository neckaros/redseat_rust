use serde::{Deserialize, Serialize};

use crate::{model::series::{ExternalImage, ExternalSerieImages}, tools::image_tools::ImageType};

use super::tmdb_configuration::TmdbConfiguration;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmdbImage {
    #[serde(rename = "aspect_ratio")]
    pub aspect_ratio: f64,
    pub height: i64,
    #[serde(rename = "iso_639_1")]
    pub iso_639_1: Option<String>,
    #[serde(rename = "file_path")]
    pub file_path: String,
    #[serde(rename = "vote_average")]
    pub vote_average: f64,
    #[serde(rename = "vote_count")]
    pub vote_count: i64,
    pub width: i64,
}


pub trait ToBest {
    fn into_best(self, lang: &Option<String>) -> Option<TmdbImage>;
    fn into_externals(self, root: &str, kind: Option<ImageType>) -> Vec<ExternalImage>;
}

impl ToBest for Vec<TmdbImage> {
    fn into_best(mut self, lang: &Option<String>) -> Option<TmdbImage> {
        self.sort_by(|a, b| b.vote_average.partial_cmp(&a.vote_average).unwrap());

        if let Some(lang) = &lang {
            let next = self.iter().filter(|i| i.iso_639_1.as_ref() == Some(lang)).next();
            if let Some(next) = next {
                return Some(next.to_owned());
            }
        }
        let default_lang = "en".to_string();
        let next = self.iter().filter(|i| i.iso_639_1.as_deref() == Some("en")).next();
        if let Some(next) = next {
            return Some(next.to_owned());
        }
        

        self.into_iter().next()
    }

    fn into_externals(self, root: &str, kind: Option<ImageType>) -> Vec<ExternalImage> {

        self.into_iter().map(|i| i.into_external(root, kind.clone())).collect()
    }

}

impl TmdbImage {
    pub fn full_path(&self, root: &str) -> String {
        format!("{}original{}", root, self.file_path)
    }

    fn into_external(self, root: &str, kind: Option<ImageType>) -> ExternalImage {
        ExternalImage {
            kind,
            url: self.full_path(root),
            aspect_ratio: Some(self.aspect_ratio),
            height: Some(self.height),
            lang: self.iso_639_1,
            vote_average: Some(self.vote_average),
            vote_count: Some(self.vote_count),
            width: Some(self.width),
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmdbImageResponse {
    pub backdrops: Option<Vec<TmdbImage>>,
    pub id: i64,
    pub logos: Option<Vec<TmdbImage>>,
    pub posters: Option<Vec<TmdbImage>>,
    pub stills: Option<Vec<TmdbImage>>,
}

impl TmdbImageResponse {
    pub fn into_external(self, configuration: &TmdbConfiguration, lang: &Option<String>) -> ExternalSerieImages {
        ExternalSerieImages {
            backdrop: self.backdrops.and_then(|l| l.into_best(lang)).and_then(|p| Some(p.full_path(&configuration.images.secure_base_url))),
            logo: self.logos.and_then(|l| l.into_best(lang)).and_then(|p| Some(p.full_path(&configuration.images.secure_base_url))),
            poster:  self.posters.and_then(|l| l.into_best(lang)).and_then(|p| Some(p.full_path(&configuration.images.secure_base_url))),
            still: self.stills.and_then(|l| l.into_best(lang)).and_then(|p| Some(p.full_path(&configuration.images.secure_base_url))),
            ..Default::default()
        }
    }

    pub fn into_externals(self, configuration: &TmdbConfiguration) -> Vec<ExternalImage> {
        let mut images = vec![];

        for image in self.backdrops {
            let mut target = image.into_externals(&configuration.images.secure_base_url, Some(ImageType::Background));
            images.append(&mut target);
        }
        for image in self.logos {
            let mut target = image.into_externals(&configuration.images.secure_base_url, Some(ImageType::ClearLogo));
            images.append(&mut target);
        }
        for image in self.posters {
            let mut target = image.into_externals(&configuration.images.secure_base_url, Some(ImageType::Poster));
            images.append(&mut target);
        }
        for image in self.stills {
            let mut target = image.into_externals(&configuration.images.secure_base_url, Some(ImageType::Still));
            images.append(&mut target);
        }
        images
    }
}