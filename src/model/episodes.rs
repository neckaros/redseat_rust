


use std::{io::Read, num, pin::Pin};

use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::{Value};
use tokio::{fs::File, io::{AsyncRead, BufReader}};


use crate::{domain::{episode::{self, Episode, EpisodesMessage}, library::LibraryRole, people::{PeopleMessage, Person}, serie::{Serie, SeriesMessage}, ElementAction}, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::image_tools::{ImageSize, ImageType}};

use super::{error::{Error, Result}, users::ConnectedUser, ModelController};


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodeForAdd {
    pub serie_ref: String,
    pub season: usize,
    pub number: usize,
    pub abs: Option<usize>,

	pub name: Option<String>,
	pub overview: Option<String>,
    pub alt: Option<Vec<String>>,

    
    pub airdate: Option<u64>,
    pub duration: Option<u64>,

    pub params: Option<Value>,
    pub imdb: Option<String>,
    pub slug: Option<String>,
    pub tmdb: Option<u64>,
    pub trakt: Option<u64>,
    pub tvdb: Option<u64>,
    pub otherids: Option<String>,
    
    pub imdb_rating: Option<f32>,
    pub imdb_votes: Option<u64>,
    pub trakt_rating: Option<f32>,
    pub trakt_votes: Option<u64>,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodeQuery {
    pub serie_ref: Option<String>,
    pub season: Option<usize>,
    pub after: Option<u64>
}

impl EpisodeQuery {
    pub fn new_empty() -> EpisodeQuery {
        EpisodeQuery { after: None, serie_ref: None, season: None }
    }
    pub fn from_after(after: u64) -> EpisodeQuery {
        EpisodeQuery { after: Some(after), serie_ref: None, season: None }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodeForUpdate {
    pub abs: Option<usize>,

	pub name: Option<String>,
	pub overview: Option<String>,
    pub alt: Option<Vec<String>>,
    pub add_alts: Option<Vec<String>>,
    pub remove_alts: Option<Vec<String>>,


    
    pub airdate: Option<u64>,
    pub duration: Option<u64>,

    pub params: Option<Value>,
    pub imdb: Option<String>,
    pub slug: Option<String>,
    pub tmdb: Option<u64>,
    pub trakt: Option<u64>,
    pub tvdb: Option<u64>,
    pub otherids: Option<String>,
    
    pub imdb_rating: Option<f32>,
    pub imdb_votes: Option<u64>,
    pub trakt_rating: Option<f32>,
    pub trakt_votes: Option<u64>,
}



impl ModelController {

	pub async fn get_episodes(&self, library_id: &str, query: EpisodeQuery, requesting_user: &ConnectedUser) -> Result<Vec<Episode>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let episodes = store.get_episodes(query).await?;
		Ok(episodes)
	}

    pub async fn get_episode(&self, library_id: &str, serie_id: String, season: usize, number: usize, requesting_user: &ConnectedUser) -> Result<Option<Episode>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let episode = store.get_episode(&serie_id, season, number).await?;
		Ok(episode)
	}

    pub async fn update_episode(&self, library_id: &str, serie_id: String, season: usize, number: usize, update: EpisodeForUpdate, requesting_user: &ConnectedUser) -> Result<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		store.update_episode(&serie_id, season, number, update).await?;
        let episode = store.get_episode(&serie_id, season, number).await?.ok_or(Error::NotFound)?;
        self.send_episode(EpisodesMessage { library: library_id.to_string(), action: ElementAction::Updated, episodes: vec![episode.clone()] });
        Ok(episode)
	}


	pub fn send_episode(&self, message: EpisodesMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("tags", message);
			}
		});
	}


    pub async fn add_episode(&self, library_id: &str, new_serie: EpisodeForAdd, requesting_user: &ConnectedUser) -> Result<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		store.add_episode(new_serie.clone()).await?;
        let new_episode = self.get_episode(library_id, new_serie.serie_ref, new_serie.season, new_serie.number, requesting_user).await?.ok_or(Error::NotFound)?;
        self.send_episode(EpisodesMessage { library: library_id.to_string(), action: ElementAction::Added, episodes: vec![new_episode.clone()] });
		Ok(new_episode)
	}


    pub async fn remove_episode(&self, library_id: &str, serie_id: &str, season: usize, number: usize, requesting_user: &ConnectedUser) -> Result<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_episode(serie_id, season, number).await?;
        if let Some(existing) = existing { 
            store.remove_episode(serie_id.to_string(), season, number).await?;
            self.send_episode(EpisodesMessage { library: library_id.to_string(), action: ElementAction::Removed, episodes: vec![existing.clone()] });
            Ok(existing)
        } else {
            Err(Error::NotFound)
        }
	}


    
	pub async fn episode_image(&self, library_id: &str, serie_id: &str, season: &usize, episode: usize, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> Result<FileStreamResult<AsyncReadPinBox>> {
        self.library_image(library_id, &format!(".series/{}", serie_id), &format!("{}.{}", season, episode), None, size, requesting_user).await
	}

    pub async fn update_episode_image<T: AsyncRead>(&self, library_id: &str, serie_id: &str, season: &usize, episode: usize, reader: T, requesting_user: &ConnectedUser) -> Result<()> {

        self.update_library_image(library_id, &format!(".series/{}", serie_id), &format!("{}.{}", season, episode), &ImageType::Still, reader, requesting_user).await
	}
    
}
