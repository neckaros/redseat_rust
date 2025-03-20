use http::header::AUTHORIZATION;
use reqwest::{Client, Request, RequestBuilder, Url};
use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, ExternalImage};

use crate::{ model::series::ExternalSerieImages, plugins::medias::tmdb::tmdb_image::{TmdbImage, ToBest}};

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
    pub async fn serie_image(&self, ids: RsIds, lang: &Option<String>) -> crate::Result<ExternalSerieImages> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("tv/{}/images", id));
        let response = request/*.query(&[("include_image_language", "en,fr")])*/.send().await?;
        let images = response.json::<TmdbImageResponse>().await?;
        //println!("images: {:?}", images);
        let bests = images.into_external(&self.configuration, lang);
        Ok(bests)
    }

    pub async fn serie_images(&self, ids: RsIds) -> crate::Result<Vec<ExternalImage>> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("tv/{}/images", id));
        let response = request/*.query(&[("include_image_language", "en,fr")])*/.send().await?;
        let images = response.json::<TmdbImageResponse>().await?;
        //println!("images: {:?}", images);
        let bests = images.into_externals(&self.configuration);
        Ok(bests)
    }

    pub async fn episode_image(&self, serie_ids: RsIds, season: &u32, episode: &u32, lang: &Option<String>) -> crate::Result<ExternalSerieImages> {

        let id = serie_ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("tv/{}/season/{}/episode/{}/images", id, season, episode));
        let response = request.send().await?;
        println!("Reponse: {:?}", response);
        let images = response.json::<TmdbImageResponse>().await?;
        //println!("images: {:?}", images);
        let bests = images.into_external(&self.configuration, lang);
        Ok(bests)
    }

    pub async fn episode_images(&self, ids: RsIds, season: &u32, episode: &u32) -> crate::Result<Vec<ExternalImage>> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("tv/{}/season/{}/episode/{}/images", id, season, episode));
        let response = request.send().await?;
        let images = response.json::<TmdbImageResponse>().await?;
        //println!("images: {:?}", images);
        let bests = images.into_externals(&self.configuration);
        Ok(bests)
    }

    pub async fn movie_image(&self, ids: RsIds, lang: &Option<String>) -> crate::Result<ExternalSerieImages> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("movie/{}/images", id));

        let response = request.send().await?;
        let images = response.json::<TmdbImageResponse>().await?;
        let bests = images.into_external(&self.configuration, lang);
        Ok(bests)
    }

    pub async fn movie_images(&self, ids: RsIds) -> crate::Result<Vec<ExternalImage>> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("movie/{}/images", id));

        let response = request.send().await?;
        let images = response.json::<TmdbImageResponse>().await?;
        let bests = images.into_externals(&self.configuration);
        Ok(bests)
    }

    pub async fn person_image(&self, ids: RsIds, lang: &Option<String>) -> crate::Result<ExternalSerieImages> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("person/{}/images", id));

        let response = request.send().await?;
        let images = response.json::<TmdbImageResponse>().await?;
        let bests = images.into_external(&self.configuration, lang);
        Ok(bests)
    }

    pub async fn person_images(&self, ids: RsIds) -> crate::Result<Vec<ExternalImage>> {
        let id = ids.try_tmdb()?;
        let request = self.get_request_builder(&format!("person/{}/images", id));

        let response = request.send().await?;
        let images = response.json::<TmdbImageResponse>().await?;
        let bests = images.into_externals(&self.configuration);
        Ok(bests)
    }
}



#[cfg(test)]
mod tests {

    use chrono::{TimeZone, Utc};
    use rs_plugin_common_interfaces::Gender;

    use crate::{error::RsResult, tools::clock::RsNaiveDate};

    use super::*;

    fn exemple_movie() -> RsIds {
        RsIds::from_imdb("tt1160419".to_owned())
    }
    #[tokio::test]
    async fn trakt_releases() -> RsResult<()> {
        let tmdb = TmdbContext::new("4a01db3a73eed5cf17e9c7c27fd9d008".to_string()).await?;
        let images = tmdb.person_images(RsIds { tmdb: Some(56731), ..Default::default()}).await?;

        println!("images {:?}", images);
        Ok(())

    }
}