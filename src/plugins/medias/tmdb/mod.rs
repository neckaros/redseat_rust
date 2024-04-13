use http::header::AUTHORIZATION;
use reqwest::{Client, Request, RequestBuilder, Url};

use crate::{domain::MediasIds, model::series::ExternalSerieImages, plugins::medias::tmdb::tmdb_image::{TmdbImage, ToBest}};

use self::{tmdb_configuration::TmdbConfiguration, tmdb_image::TmdbImageResponse};

pub mod tmdb_image;
pub mod tmdb_configuration;

#[derive(Debug, Clone)]
pub struct TmdbContext {
    base_url: Url,
    client_id: String,
    client: Client,
    configuration: TmdbConfiguration
}

impl TmdbContext {
    pub async fn new(client_id: String) -> crate::Result<Self> {
        let base_url = reqwest::Url::parse("https://api.themoviedb.org/3/").unwrap();
        let client = reqwest::Client::new();
        let config_url = base_url.join("configuration").unwrap();
        let response = client.get(config_url).query(&[("api_key", &client_id)]).send().await?;
        let configuration = response.json::<TmdbConfiguration>().await?;

        Ok(TmdbContext {
            base_url, //"https://api.trakt.tv".to_string(),
            client_id,
            client: reqwest::Client::new(),
            configuration
        })
    }
}

impl MediasIds {
    pub fn try_tmdb(self) -> crate::Result<u64> {
        self.tmdb.ok_or(crate::Error::NoMediaIdRequired(Box::new(self.clone())))
    }
}

impl TmdbContext {
    pub fn add_auth(&self, request: RequestBuilder) -> RequestBuilder {
        request.query(&[("api_key", &self.client_id)])
    }
    pub fn get_request_builder(&self, path: &str) -> RequestBuilder{
        let base_url = reqwest::Url::parse("https://api.themoviedb.org/3/").unwrap();
        let config_url = base_url.join(path).unwrap();
        let builder = self.client.get(config_url);
        self.add_auth(builder)
    }
}


impl TmdbContext {
    pub async fn serie_image(&self, ids: MediasIds) -> crate::Result<ExternalSerieImages> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("tv/{}/images", id));
        let response = request.query(&[("include_image_language", "en,fr")]).send().await?;
        let images = response.json::<TmdbImageResponse>().await?;
        //println!("images: {:?}", images);
        let bests = images.into_external(&self.configuration);
        Ok(bests)
    }

    pub async fn episode_image(&self, ids: MediasIds, season: &u32, episode: &u32) -> crate::Result<ExternalSerieImages> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("tv/{}/season/{}/episode/{}/images", id, season, episode));
        let response = request.send().await?;
        let images = response.json::<TmdbImageResponse>().await?;
        //println!("images: {:?}", images);
        let bests = images.into_external(&self.configuration);
        Ok(bests)
    }

    pub async fn movie_image(&self, ids: MediasIds) -> crate::Result<ExternalSerieImages> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("movie/{}/images", id));

        let response = request.query(&[("include_image_language", "en,fr")]).send().await?;
        let images = response.json::<TmdbImageResponse>().await?;
        let bests = images.into_external(&self.configuration);
        Ok(bests)
    }
}