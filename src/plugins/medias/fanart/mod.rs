
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};

use crate::{domain::MediasIds, model::series::ExternalSerieImages};


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
impl From<FanArtMovieResult> for ExternalSerieImages {
    fn from(value: FanArtMovieResult) -> Self {
        ExternalSerieImages {
            backdrop: None,
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

impl MediasIds {
    fn try_tvdb(self) -> crate::Result<u64> {
        self.tvdb.ok_or(crate::Error::NoMediaIdRequired(Box::new(self.clone())))
    }
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

    pub async fn serie_image(&self, ids: MediasIds) -> crate::Result<ExternalSerieImages> {
        let id = ids.try_tvdb()?;
        let request = self.get_request_builder(&format!("tv/{}", id));
        let response = request.send().await?;
        let images = response.json::<FanArtSerieResult>().await?;
        
        let bests: ExternalSerieImages = images.into();
        Ok(bests)
    }


    pub async fn movie_image(&self, ids: MediasIds) -> crate::Result<ExternalSerieImages> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("movies/{}", id));

        let response = request.send().await?;
        let images = response.json::<FanArtMovieResult>().await?;
        let bests:ExternalSerieImages = images.into();
        Ok(bests)
    }

}