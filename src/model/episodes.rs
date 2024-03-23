


use std::{io::{self, Read}, num, pin::Pin};

use async_recursion::async_recursion;
use futures::TryStreamExt;
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::{Value};
use tokio::{fs::File, io::{AsyncRead, AsyncWriteExt, BufReader}};
use tokio_util::io::StreamReader;


use crate::{domain::{episode::{self, Episode, EpisodeWithShow, EpisodesMessage}, library::LibraryRole, people::{PeopleMessage, Person}, serie::{self, Serie, SeriesMessage}, ElementAction, MediasIds}, error::RsResult, plugins::sources::{AsyncReadPinBox, FileStreamResult, Source}, tools::{image_tools::{resize_image_reader, ImageSize, ImageType}, log::log_info}};

use super::{error::{Error, Result}, users::ConnectedUser, ModelController};


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct EpisodeQuery {
    pub serie_ref: Option<String>,
    pub season: Option<u32>,
    pub after: Option<u64>,
    pub limit: Option<u32>
}

impl EpisodeQuery {
    pub fn new_empty() -> EpisodeQuery {
        EpisodeQuery { ..Default::default() }
    }
    pub fn from_after(after: u64) -> EpisodeQuery {
        EpisodeQuery { after: Some(after), ..Default::default() }
    }

    pub fn limit_or_default(&self) -> u32 {
        self.limit.unwrap_or(200)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodeForUpdate {
    pub abs: Option<u32>,

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

    pub async fn get_episodes_upcoming(&self, library_id: &str, query: EpisodeQuery, requesting_user: &ConnectedUser) -> Result<Vec<Episode>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let episodes = store.get_episodes_upcoming(query).await?;
		Ok(episodes)
	}

    pub async fn get_episode(&self, library_id: &str, serie_id: String, season: u32, number: u32, requesting_user: &ConnectedUser) -> Result<Option<Episode>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let episode = store.get_episode(&serie_id, season, number).await?;
		Ok(episode)
	}

    pub async fn update_episode(&self, library_id: &str, serie_id: String, season: u32, number: u32, update: EpisodeForUpdate, requesting_user: &ConnectedUser) -> Result<Episode> {
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


    pub async fn add_episode(&self, library_id: &str, new_serie: Episode, requesting_user: &ConnectedUser) -> Result<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		store.add_episode(new_serie.clone()).await?;
        let new_episode = self.get_episode(library_id, new_serie.serie_ref, new_serie.season, new_serie.number, requesting_user).await?.ok_or(Error::NotFound)?;
        self.send_episode(EpisodesMessage { library: library_id.to_string(), action: ElementAction::Added, episodes: vec![new_episode.clone()] });
		Ok(new_episode)
	}


    pub async fn remove_episode(&self, library_id: &str, serie_id: &str, season: u32, number: u32, requesting_user: &ConnectedUser) -> Result<Episode> {
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

    pub async fn get_serie_ids(&self, library_id: &str, serie_id: &str, requesting_user: &ConnectedUser) -> RsResult<MediasIds> {
        let serie = self.get_serie(library_id, serie_id.to_string(), requesting_user).await?.ok_or(Error::NotFound)?;
        let ids: MediasIds = serie.into();
        Ok(ids)
    }

    pub async fn refresh_episodes(&self, library_id: &str, serie_id: &str, requesting_user: &ConnectedUser) -> RsResult<Vec<Episode>> {
        let ids = self.get_serie_ids(library_id, serie_id, requesting_user).await?;
        let all_episodes = self.trakt.all_episodes(&ids).await?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        store.remove_all_serie_episodes(serie_id.to_string()).await?;
        let mut new_episodes: Vec<Episode> = vec![];
        for episode in all_episodes {
            let episode = self.add_episode(library_id, episode, requesting_user).await?;
            new_episodes.push(episode)
        }
        Ok(new_episodes)
    }


    #[async_recursion]
	pub async fn episode_image(&self, library_id: &str, serie_id: &str, season: &u32, episode: &u32, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        if MediasIds::is_id(&serie_id) {
            let mut serie_ids: MediasIds = serie_id.to_string().try_into()?;

            let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
            let existing_serie = store.get_serie_by_external_id(serie_ids.clone()).await?;
            if let Some(existing_serie) = existing_serie {
                self.episode_image(library_id, &existing_serie.id, season,  episode, size, requesting_user).await
            } else {

                let local_provider = self.library_source_for_library(library_id).await?;
                
                if serie_ids.tmdb.is_none() {
                    let serie = self.trakt.get_serie(&serie_ids).await?;
                    serie_ids = serie.into();
                }
                let image_path = format!("cache/serie-{}-episode-{}x{}.webp", serie_id.replace(":", "-"), season, episode);

                if !local_provider.exists(&image_path).await {
                    let (_, mut writer) = local_provider.get_file_write_stream(&image_path).await?;
                    let images = self.tmdb.episode_image(serie_ids, season, episode).await?.into_kind(ImageType::Still).ok_or(crate::Error::NotFound)?;
                    let image_reader = reqwest::get(images).await?;
                    let stream = image_reader.bytes_stream();
                    let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
                    let mut body_reader = StreamReader::new(body_with_io_error);
                    let resized = resize_image_reader(&mut body_reader, ImageSize::Large.to_size()).await?;

                    writer.write_all(&resized).await?;
                }

                let source = local_provider.get_file(&image_path, None).await?;
                match source {
                    crate::plugins::sources::SourceRead::Stream(s) => Ok(s),
                    crate::plugins::sources::SourceRead::Request(_) => Err(crate::Error::GenericRedseatError),
                }
            }
        } else {
            if !self.has_library_image(library_id, ".series", serie_id, None, requesting_user).await? {
                log_info(crate::tools::log::LogServiceType::Source, format!("Updating episode image: {}", serie_id));
                self.refresh_episode_image(library_id, serie_id, season, episode, requesting_user).await?;
            }
            
            let image = self.library_image(library_id, &format!(".series/{}", serie_id), &format!("{}.{}", season, episode), None, size, requesting_user).await?;
            Ok(image)
        }
        
	}

    /// download and update image
    pub async fn refresh_episode_image(&self, library_id: &str, serie_id: &str, season: &u32, episode: &u32, requesting_user: &ConnectedUser) -> RsResult<()> {
        let ids: MediasIds = self.get_serie_ids(library_id, serie_id, requesting_user).await?;
        let reader = self.download_episode_image(&ids, season, episode).await?;
        self.update_episode_image(library_id, serie_id, season, episode, reader, requesting_user).await?;
        Ok(())
    }
    pub async fn download_episode_image(&self, ids: &MediasIds, season: &u32, episode: &u32) -> crate::Result<AsyncReadPinBox> {
        let images = self.tmdb.episode_image(ids.clone(), season, episode).await?.into_kind(ImageType::Still).ok_or(crate::Error::NotFound)?;
        let image_reader = reqwest::get(images).await?;
        let stream = image_reader.bytes_stream();
        let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
        let body_reader = StreamReader::new(body_with_io_error);
        Ok(Box::pin(body_reader))
    }

    pub async fn update_episode_image<T: AsyncRead>(&self, library_id: &str, serie_id: &str, season: &u32, episode: &u32, reader: T, requesting_user: &ConnectedUser) -> Result<()> {
        self.update_library_image(library_id, &format!(".series/{}", serie_id), &format!("{}.{}", season, episode), &None, reader, requesting_user).await
	}
    
}
