use reqwest::{Client, Url};
use tower::Service;
use crate::{domain::{episode::Episode, serie::Serie, MediasIds}, model::series::SerieForAdd, plugins::medias::trakt::{trakt_episode::TraktSeasonWithEpisodes, trakt_show::TraktFullShow}, Error, Result};

use self::{trakt_episode::TraktFullEpisode, trakt_show::TraktTrendingShowResult};
// Context required for all requests

mod trakt_show;
mod trakt_episode;

#[derive(Debug, Clone)]
pub struct TraktContext {
    base_url: Url,
    client_id: String,
    client: Client
}

impl TraktContext {
    pub fn new(client_id: String) -> Self {
        let base_url = reqwest::Url::parse("https://api.trakt.tv").unwrap();
        TraktContext {
            base_url, //"https://api.trakt.tv".to_string(),
            client_id,
            client: reqwest::Client::new()
        }
    }
}

impl TraktContext {
    pub async fn get_serie(&self, id: &MediasIds) -> crate::Result<Serie> {

        let id = id.as_id_for_trakt().ok_or(Error::NoMediaIdRequired(id.clone()))?;

        let url = self.base_url.join(&format!("shows/{}?extended=full", id)).unwrap();
        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let show = r.json::<TraktFullShow>().await?;
        let show_nous: Serie = show.into();
        Ok(show_nous)
    }

    pub async fn trending_shows(&self) -> crate::Result<Vec<Serie>> {
        let url = self.base_url.join("shows/trending?extended=full").unwrap();
        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let shows: Vec<Serie> = r.json::<Vec<TraktTrendingShowResult>>().await?.into_iter().map(|s| s.show).map(Serie::from).collect();
        Ok(shows)
    }

    pub async fn all_episodes(&self, id: &MediasIds) -> crate::Result<Vec<Episode>> {
        let serie_id = id.redseat.as_ref().ok_or(Error::NotFound)?;
        let id = if let Some(imdb) = &id.imdb {
            Ok(imdb.to_string())
        } else if let Some(trakt) = &id.trakt {
            Ok(trakt.to_string())
        } else {
            Err(Error::NoMediaIdRequired(id.clone()))
        }?;
        let url = self.base_url.join(&format!("shows/{}/seasons?extended=full,episodes", id)).unwrap();
        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let episodes = r.json::<Vec<TraktSeasonWithEpisodes>>().await?.into_iter().flat_map(|s| s.episodes).map(|e| e.into_trakt(serie_id.clone())).collect::<Vec<_>>();
        Ok(episodes)
    }

    pub async fn episode(&self, id: &MediasIds, season: u32, episode: u32) -> crate::Result<Episode> {

        let id = if let Some(imdb) = &id.imdb {
            Ok(imdb.to_string())
        } else if let Some(trakt) = &id.trakt {
            Ok(trakt.to_string())
        } else {
            Err(Error::NoMediaIdRequired(id.clone()))
        }?;
        let url = self.base_url.join(&format!("shows/{}/seasons/{}/episodes/{}?extended=full", id, season, episode)).unwrap();
        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let episodes = r.json::<TraktFullEpisode>().await?;
        Ok(episodes.into_trakt(format!("trakt:")))
    }
}

async fn get_movie() -> Result<()> {
    // Create a request and convert it into an HTTP request

    Ok(())

}