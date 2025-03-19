
use reqwest::{Client, RequestBuilder};
use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, ExternalImage, ImageType};
use serde::{Deserialize, Serialize};

use crate::model::series::ExternalSerieImages;


#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanArtImage {
    pub url: String,
    pub lang: String,
    pub likes: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanArtSerieResult {
    pub hdtvlogo: Option<Vec<FanArtImage>>,
    pub tvposter: Option<Vec<FanArtImage>>,
    pub tvbanner: Option<Vec<FanArtImage>>,
    pub showbackground: Option<Vec<FanArtImage>>,
    pub tvthumb: Option<Vec<FanArtImage>>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanArtMovieResult {
    pub hdmovieclearart: Option<Vec<FanArtImage>>,
    pub moviethumb: Option<Vec<FanArtImage>>,
    pub moviebanner: Option<Vec<FanArtImage>>,
    pub movieposter: Option<Vec<FanArtImage>>,
    pub hdmovielogo: Option<Vec<FanArtImage>>,
    pub movielogo: Option<Vec<FanArtImage>>,
    pub moviebackground: Option<Vec<FanArtImage>>,
    
}

impl From<FanArtSerieResult> for ExternalSerieImages {
    fn from(value: FanArtSerieResult) -> Self {
        ExternalSerieImages {
            backdrop: value.showbackground.into_best().map(|i| i.url),
            logo: value.hdtvlogo.into_best().map(|i| i.url),
            poster: value.tvposter.into_best().map(|i| i.url),
            still: None,
            card: value.tvthumb.into_best().map(|i| i.url),
        }
    }
}




impl FanArtImage {
    fn into_external(self, kind: Option<ImageType>) -> ExternalImage {
        ExternalImage {
            kind,
            url: self.url,
            aspect_ratio: None,
            height: None,
            lang: Some(self.lang),
            vote_average: None,
            vote_count: None,
            width: None,
        }
    }
}

pub trait FanArtToExternals {
    fn into_externals(self, kind: Option<ImageType>) -> Vec<ExternalImage>;
}

impl FanArtToExternals for Vec<FanArtImage> {
    fn into_externals(self, kind: Option<ImageType>) -> Vec<ExternalImage> {
        self.into_iter().map(|i| i.into_external(kind.clone())).collect()
    }
}

impl From<FanArtSerieResult> for Vec<ExternalImage> {
    fn from(value: FanArtSerieResult) -> Self {
        let mut images = vec![];

        if let Some(result_images) = value.showbackground {
            let mut target = result_images.into_externals(Some(ImageType::Background));
            images.append(&mut target);
        }
        if let Some(result_images) = value.hdtvlogo {
            let mut target = result_images.into_externals(Some(ImageType::ClearLogo));
            images.append(&mut target);
        }
        if let Some(result_images) = value.tvposter {
            let mut target = result_images.into_externals(Some(ImageType::Poster));
            images.append(&mut target);
        }
        if let Some(result_images) = value.tvthumb {
            let mut target = result_images.into_externals(Some(ImageType::Card));
            images.append(&mut target);
        }
        
        images
    }
}


impl From<FanArtMovieResult> for Vec<ExternalImage> {
    fn from(value: FanArtMovieResult) -> Self {
        let mut images = vec![];

        if let Some(result_images) = value.moviebackground {
            let mut target = result_images.into_externals(Some(ImageType::Background));
            images.append(&mut target);
        }
        if let Some(result_images) = value.movielogo {
            let mut target = result_images.into_externals(Some(ImageType::ClearLogo));
            images.append(&mut target);
        }
        if let Some(result_images) = value.movieposter {
            let mut target = result_images.into_externals(Some(ImageType::Poster));
            images.append(&mut target);
        }
        if let Some(result_images) = value.moviethumb {
            let mut target = result_images.into_externals(Some(ImageType::Card));
            images.append(&mut target);
        }
        
        images
    }
}

impl From<FanArtMovieResult> for ExternalSerieImages {
    fn from(value: FanArtMovieResult) -> Self {
        ExternalSerieImages {
            backdrop: value.moviebackground.into_best().map(|i| i.url),
            logo: value.movielogo.into_best().map(|i| i.url),
            poster: value.movieposter.into_best().map(|i| i.url),
            still: None,
            card: value.moviethumb.into_best().map(|i| i.url),
        }
    }
}


pub trait ToBest {
    fn into_best(self) -> Option<FanArtImage>;
}

impl ToBest for Option<Vec<FanArtImage>> {
    fn into_best(self) -> Option<FanArtImage> {
        if let Some(mut v) = self {
            v.sort_by(|a, b| b.likes.partial_cmp(&a.likes).unwrap_or(std::cmp::Ordering::Less));
            v.into_iter().next()
        } else {
            None
        }
        
    }
}


#[derive(Debug, Clone)]
pub struct FanArtContext {
    token: String,
    client: Client,
}

impl FanArtContext {
    pub fn add_auth(&self, request: RequestBuilder) -> RequestBuilder {
        request.query(&[("api_key", &self.token)])
    }
    pub fn get_request_builder(&self, path: &str) -> RequestBuilder{
        let base_url = reqwest::Url::parse("https://webservice.fanart.tv/v3/").unwrap();
        let config_url = base_url.join(path).unwrap();
        let builder = self.client.get(config_url);
        self.add_auth(builder)
    }
}

impl FanArtContext {
    pub fn new(token: String) -> Self {
        FanArtContext {
            token,
            client: reqwest::Client::new()
        }
    }

    pub async fn serie_image(&self, ids: RsIds) -> crate::Result<ExternalSerieImages> {
        let id = ids.try_tvdb()?;
        let request = self.get_request_builder(&format!("tv/{}", id));
        let response = request.send().await?;
        let images = response.json::<FanArtSerieResult>().await?;
        
        let bests: ExternalSerieImages = images.into();
        Ok(bests)
    }
    pub async fn serie_images(&self, ids: RsIds) -> crate::Result<Vec<ExternalImage>> {
        let id = ids.try_tvdb()?;
        let request = self.get_request_builder(&format!("tv/{}", id));
        let response = request.send().await?;
        let images = response.json::<FanArtSerieResult>().await?;
        
        let bests: Vec<ExternalImage> = images.into();
        Ok(bests)
    }

    pub async fn movie_image(&self, ids: RsIds) -> crate::Result<ExternalSerieImages> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("movies/{}", id));

        let response = request.send().await?;
        let images = response.json::<FanArtMovieResult>().await?;
        let bests:ExternalSerieImages = images.into();
        Ok(bests)
    }

    pub async fn movie_images(&self, ids: RsIds) -> crate::Result<Vec<ExternalImage>> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("movies/{}", id));

        let response = request.send().await?;
        let images = response.json::<FanArtMovieResult>().await?;
        let bests: Vec<ExternalImage> = images.into();
        Ok(bests)
    }

}