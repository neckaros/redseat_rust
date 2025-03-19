use chrono::{DateTime, FixedOffset};
use reqwest::{Client, Url};
use rs_plugin_common_interfaces::{domain::rs_ids::{RsIds, RsIdsError}, lookup::RsLookupMovie};
use tower::Service;
use crate::{domain::{episode::Episode, movie::Movie, serie::Serie}, plugins::medias::trakt::{trakt_episode::TraktSeasonWithEpisodes, trakt_show::TraktFullShow}, tools::clock::{Clock, RsNaiveDate}, Error, Result};

use self::{trakt_episode::TraktFullEpisode, trakt_movie::{TraktFullMovie, TraktMovieSearchElement, TraktRelease, TraktReleaseType, TraktReleases, TraktTrendingMoviesResult}, trakt_show::{TraktShowSearchElement, TraktTrendingShowResult}};
// Context required for all requests
use unidecode::unidecode;

mod trakt_show;
mod trakt_episode;
mod trakt_movie;
mod trakt_people;

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

    pub async fn episodes_refreshed(&self, date: DateTime<FixedOffset>) -> crate::Result<Vec<u64>> {
        let mut all_updates:Vec<u64> = vec![];
        let mut page = 1;
        loop {
            let url = self.base_url.join(&format!("shows/updates/id/{}?page={}&limit=100", date.to_utc().fixed_offset().print(), page)).unwrap();
            let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
            let nb_pages: u32 = r.headers().get("x-pagination-page-count").ok_or(crate::Error::TraktTooManyUpdates)?.to_str().map_err(|_| crate::Error::TraktTooManyUpdates)?.parse().map_err(|_| crate::Error::TraktTooManyUpdates)?;
            let mut updates: Vec<u64> = r.json::<Vec<u64>>().await?.into_iter().collect();
            all_updates.append(&mut updates);
            if nb_pages > 10 {
                return Err(crate::Error::TraktTooManyUpdates.into());
            } else if page < nb_pages {
                page = page + 1;
            } else {
                break;
            }
        }
        Ok(all_updates)
    }

    pub async fn get_serie(&self, id: &RsIds) -> crate::Result<Serie> {

        let id = id.as_id_for_trakt().ok_or(RsIdsError::NoMediaIdRequired(Box::new(id.clone())))?;

        let url = self.base_url.join(&format!("shows/{}?extended=full", id)).unwrap();
        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let show = r.json::<TraktFullShow>().await?;
        
        let show_nous: Serie = show.into();
        Ok(show_nous)
    }

    pub async fn search_show(&self, search: &RsLookupMovie) -> crate::Result<Vec<Serie>> {

        let url = self.base_url.join(&format!("search/show?extended=full&query={}", unidecode(&search.name))).unwrap();

        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;

        let shows: Vec<Serie> = r.json::<Vec<TraktShowSearchElement>>().await?.into_iter().map(|m| Serie::from(m.show)).collect();
      
        Ok(shows)
    }


    pub async fn trending_shows(&self) -> crate::Result<Vec<Serie>> {
        let url = self.base_url.join("shows/trending?extended=full").unwrap();
        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let shows: Vec<Serie> = r.json::<Vec<TraktTrendingShowResult>>().await?.into_iter().map(|s| s.show).map(Serie::from).collect();
        Ok(shows)
    }

    pub async fn all_episodes(&self, id: &RsIds) -> crate::Result<Vec<Episode>> {
        let serie_id = id.clone().as_id_for_trakt().ok_or(Error::Error(format!("Unable to request trakt. No imdb or trakt id for: {:?}", id)))?;
        let url = self.base_url.join(&format!("shows/{}/seasons?extended=full,episodes", serie_id)).unwrap();
        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let best_serie_id = id.clone().into_best().unwrap_or(serie_id.to_owned());
        let episodes = r.json::<Vec<TraktSeasonWithEpisodes>>().await?.into_iter().flat_map(|s| s.episodes).map(|e| e.into_trakt(best_serie_id.clone())).collect::<Vec<_>>();
        Ok(episodes)
    }

    pub async fn episode(&self, id: &RsIds, season: u32, episode: u32) -> crate::Result<Episode> {

        let id = if let Some(imdb) = &id.imdb {
            Ok(imdb.to_string())
        } else if let Some(trakt) = &id.trakt {
            Ok(trakt.to_string())
        } else {
            Err(RsIdsError::NoMediaIdRequired(Box::new(id.clone())))
        }?;
        let url = self.base_url.join(&format!("shows/{}/seasons/{}/episodes/{}?extended=full", id, season, episode)).unwrap();
        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let episodes = r.json::<TraktFullEpisode>().await?;
        Ok(episodes.into_trakt(format!("trakt:")))
    }
}


impl TraktContext {

    pub async fn movies_refreshed(&self, date: DateTime<FixedOffset>) -> crate::Result<Vec<u64>> {
        let mut all_updates:Vec<u64> = vec![];
        let mut page = 1;
        loop {
            let url = self.base_url.join(&format!("movies/updates/id/{}?page={}&limit=100", date.to_utc().fixed_offset().print(), page)).unwrap();
            let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
            let nb_pages: u32 = r.headers().get("x-pagination-page-count").ok_or(crate::Error::TraktTooManyUpdates)?.to_str().map_err(|_| crate::Error::TraktTooManyUpdates)?.parse().map_err(|_| crate::Error::TraktTooManyUpdates)?;
            let mut updates: Vec<u64> = r.json::<Vec<u64>>().await?.into_iter().collect();
            all_updates.append(&mut updates);
            if nb_pages > 10 {
                return Err(crate::Error::TraktTooManyUpdates.into());
            } else if page < nb_pages {
                page = page + 1;
            } else {
                break;
            }
        }
        Ok(all_updates)
    }

    pub async fn get_movie_releases(&self, id: &RsIds) -> crate::Result<Vec<TraktRelease>> {

        let id = id.as_id_for_trakt().ok_or(RsIdsError::NoMediaIdRequired(Box::new(id.clone())))?;

        let url = self.base_url.join(&format!("movies/{}/releases", id)).unwrap();

        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let releases = r.json::<Vec<TraktRelease>>().await?;
        Ok(releases)
    }

    pub async fn get_movie(&self, ids: &RsIds) -> crate::Result<Movie> {

        let id = ids.as_id_for_trakt().ok_or(RsIdsError::NoMediaIdRequired(Box::new(ids.clone())))?;

        let url = self.base_url.join(&format!("movies/{}?extended=full", id)).unwrap();

        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let movie = r.json::<TraktFullMovie>().await?;
        let mut movie_nous: Movie = movie.into();
        let releases = self.get_movie_releases(&ids).await?;
        let digital = releases.earliest_for(TraktReleaseType::Digital);
        if digital.is_some() {
            movie_nous.digitalairdate = digital.and_then(|t| Some(t.utc().ok()?.timestamp_millis()));
        }
        let theatrical = releases.earliest_for(TraktReleaseType::Theatrical);
        if theatrical.is_some() {
            movie_nous.airdate = theatrical.and_then(|t| Some(t.utc().ok()?.timestamp_millis()));
        }
        Ok(movie_nous)
    }

    pub async fn search_movie(&self, search: &RsLookupMovie) -> crate::Result<Vec<Movie>> {

        let url = self.base_url.join(&format!("search/movie?extended=full&query={}", unidecode(&search.name))).unwrap();

        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let movies: Vec<Movie> = r.json::<Vec<TraktMovieSearchElement>>().await?.into_iter().map(|m| Movie::from(m.movie)).collect();
      
        Ok(movies)
    }



    pub async fn trending_movies(&self) -> crate::Result<Vec<Movie>> {
        let url = self.base_url.join("movies/trending?extended=full").unwrap();
        let r = self.client.get(url).header("trakt-api-key", &self.client_id).send().await?;
        let shows: Vec<Movie> = r.json::<Vec<TraktTrendingMoviesResult>>().await?.into_iter().map(|s| s.movie).map(Movie::from).collect();
        Ok(shows)
    }

}



#[cfg(test)]
mod tests {

    use chrono::{TimeZone, Utc};
    use tests::trakt_movie::{TraktReleaseType, TraktReleases};

    use crate::{error::RsResult, tools::clock::RsNaiveDate};

    use super::*;

    fn exemple_movie() -> RsIds {
        RsIds::from_imdb("tt1160419".to_owned())
    }
    #[tokio::test]
    async fn trakt_releases() -> RsResult<()> {
        let trakt = TraktContext::new("455f81b3409a8dd140a941e9250ff22b2ed92d68003491c3976363fe752a9024".to_owned());

        let releases = trakt.get_movie_releases(&exemple_movie()).await?;
        
        println!("{:?}", releases);
        println!("{:?}", releases.earliest_for(TraktReleaseType::Digital).and_then(|d| d.utc().ok()));
        Ok(())
    }
}