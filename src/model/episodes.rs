


use std::{collections::HashMap, io::{self, Read}, num, pin::Pin};

use async_recursion::async_recursion;
use futures::TryStreamExt;
use nanoid::nanoid;
use rs_plugin_common_interfaces::MediaType;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{fs::File, io::{AsyncRead, AsyncWriteExt, BufReader}};
use tokio_util::io::StreamReader;


use crate::{domain::{deleted::RsDeleted, episode::{self, Episode, EpisodeWithAction, EpisodeWithShow, EpisodesMessage}, library::LibraryRole, people::{PeopleMessage, Person}, serie::{self, Serie, SeriesMessage}, ElementAction, MediasIds}, error::RsResult, plugins::{medias::imdb::ImdbContext, sources::{AsyncReadPinBox, FileStreamResult, Source}}, tools::{array_tools::Dedup, clock::now, image_tools::{resize_image_reader, ImageSize, ImageType}, log::log_info}};

use super::{error::{Error, Result}, medias::{RsSort, RsSortOrder}, store::sql::SqlOrder, users::{ConnectedUser, HistoryQuery}, ModelController};


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeQuery {
    pub serie_ref: Option<String>,
    pub season: Option<u32>,
    
    #[serde(default)]
    pub not_seasons: Vec<u32>,

    pub after: Option<i64>,

    pub aired_before: Option<i64>,
    pub aired_after: Option<i64>,

    #[serde(default)]
    pub sorts: Vec<RsSortOrder>,

    pub limit: Option<u32>
}

impl EpisodeQuery {
    pub fn new_empty() -> EpisodeQuery {
        EpisodeQuery { ..Default::default() }
    }
    pub fn from_after(after: i64) -> EpisodeQuery {
        EpisodeQuery { after: Some(after), ..Default::default() }
    }

    pub fn limit_or_default(&self) -> u32 {
        self.limit.unwrap_or(200)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct EpisodeForUpdate {
    pub abs: Option<u32>,

	pub name: Option<String>,
	pub overview: Option<String>,
    pub alt: Option<Vec<String>>,
    pub add_alts: Option<Vec<String>>,
    pub remove_alts: Option<Vec<String>>,


    
    pub airdate: Option<i64>,
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

impl Episode {
    pub async fn fill_imdb_ratings(&mut self, imdb_context: &ImdbContext) {
        if let Some(imdb) = &self.imdb {
            let rating = imdb_context.get_rating(imdb).await.unwrap_or(None);
            if let Some(rating) = rating {
                self.imdb_rating = Some(rating.0);
                self.imdb_votes = Some(rating.1);
            }
        }
    } 
}

impl ModelController {

	pub async fn get_episodes(&self, library_id: &str, query: EpisodeQuery, requesting_user: &ConnectedUser) -> RsResult<Vec<Episode>> {
        if let Some(serie_id) = &query.serie_ref {
            return self.get_episodes_by_id(library_id, serie_id.to_owned(), query, requesting_user).await;
        }
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let mut episodes = store.get_episodes(query).await?;

        self.fill_episodes_watched_imdb(&mut episodes, requesting_user, Some(library_id.to_string())).await?;
		Ok(episodes)
	}

    pub async fn get_episodes_by_id(&self, library_id: &str, serie_id: String, mut query: EpisodeQuery, requesting_user: &ConnectedUser) -> RsResult<Vec<Episode>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let mut episodes = if MediasIds::is_id(&serie_id) {
            let id: MediasIds = serie_id.try_into().map_err(|_| Error::NotFound)?;
            let serie = store.get_serie_by_external_id(id.clone()).await?;

            if let Some(serie) = serie {
                query.serie_ref = Some(serie.id);
                store.get_episodes(query).await?
            } else {
                self.trakt.all_episodes(&id).await.map_err(|_| Error::NotFound)?
                
            }
        } else {
            store.get_episodes(query).await?
        };

        self.fill_episodes_watched_imdb(&mut episodes, requesting_user, Some(library_id.to_string())).await?;
		Ok(episodes)
	}

    pub async fn fill_episode_watched_imdb(&self, episode: &mut Episode, requesting_user: &ConnectedUser, library_id: Option<String>) -> RsResult<()> {
        let ids: MediasIds = episode.clone().into();
        let watched = self.get_watched(HistoryQuery { types: vec![MediaType::Episode], id: Some(ids.clone()), ..Default::default() }, requesting_user, library_id.clone()).await?; 
        let progress = self.get_view_progress(ids, requesting_user, library_id).await?;
        if let Some(progress) = progress {
            episode.progress = Some(progress.progress);
        }
        let watched = watched.first();
        if let Some(watched) = watched {
            episode.watched = Some(watched.date);
        }
        episode.fill_imdb_ratings(&self.imdb).await;
        Ok(())
    }
    pub async fn fill_episodes_watched_imdb(&self, episodes: &mut Vec<Episode>, requesting_user: &ConnectedUser, library_id: Option<String>) -> RsResult<()> {
        let watched = self.get_watched(HistoryQuery { types: vec![MediaType::Episode], ..Default::default() }, requesting_user, library_id.clone()).await?.into_iter().map(|e| (e.id, e.date)).collect::<HashMap<_, _>>();
        let progresses = self.get_all_view_progress(HistoryQuery { types: vec![MediaType::Episode], ..Default::default() }, requesting_user, library_id).await?.into_iter().map(|e| (e.id, e.progress)).collect::<HashMap<_, _>>();

        for episode in episodes {
            let ids = MediasIds::from(episode.clone());
            let ids_string: Vec<String> = ids.into();
            for id in ids_string {
                let watch = watched.get(&id);
                if let Some(watch) = watch {
                    episode.watched = Some(*watch);
                }
                let progress = progresses.get(&id);
                if let Some(progress) = progress {
                    episode.progress = Some(*progress);
                }
            }
            episode.fill_imdb_ratings(&self.imdb).await;
        }
        Ok(())
    }

    pub async fn get_episodes_upcoming(&self, library_id: &str, query: EpisodeQuery, requesting_user: &ConnectedUser) -> RsResult<Vec<Episode>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let mut episodes = store.get_episodes_upcoming(query).await?;
        self.fill_episodes_watched_imdb(&mut episodes, requesting_user, Some(library_id.to_string())).await?;
		Ok(episodes)
	}

    pub async fn get_episodes_ondeck(&self, library_id: &str, query: EpisodeQuery, requesting_user: &ConnectedUser) -> RsResult<Vec<Episode>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let mut episodes = store.get_episodes_aired(query).await?;
        self.fill_episodes_watched_imdb(&mut episodes, requesting_user, Some(library_id.to_string())).await?;
		let mut episodes = episodes.into_iter().filter(|e| e.watched.is_none()).collect::<Vec<_>>().dedup_key(|e| e.serie.clone());
        episodes.reverse();
		Ok(episodes)
	}

    pub async fn get_episode(&self, library_id: &str, serie_id: String, season: u32, number: u32, requesting_user: &ConnectedUser) -> RsResult<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let mut episode = store.get_episode(&serie_id, season, number).await?.ok_or(Error::NotFound)?;
        self.fill_episode_watched_imdb(&mut episode, requesting_user, Some(library_id.to_string())).await?;
		Ok(episode)
	}

    pub async fn update_episode(&self, library_id: &str, serie_id: String, season: u32, number: u32, update: EpisodeForUpdate, requesting_user: &ConnectedUser) -> RsResult<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		store.update_episode(&serie_id, season, number, update).await?;
        let episode = self.get_episode(library_id, serie_id, season, number, requesting_user).await?;
        self.send_episode(EpisodesMessage { library: library_id.to_string(), episodes: vec![EpisodeWithAction {action: ElementAction::Updated, episode: episode.clone()}] });
        Ok(episode)
	}


	pub fn send_episode(&self, message: EpisodesMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("episodes", message);
			}
		});
	}


    pub async fn add_episode(&self, library_id: &str, new_serie: Episode, requesting_user: &ConnectedUser) -> RsResult<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		store.add_episode(new_serie.clone()).await?;
        let new_episode = self.get_episode(library_id, new_serie.serie, new_serie.season, new_serie.number, requesting_user).await?;
        self.send_episode(EpisodesMessage { library: library_id.to_string(), episodes: vec![EpisodeWithAction {action: ElementAction::Added, episode: new_episode.clone()}] });
		Ok(new_episode)
	}


    pub async fn remove_episode(&self, library_id: &str, serie_id: &str, season: u32, number: u32, requesting_user: &ConnectedUser) -> RsResult<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_episode(serie_id, season, number).await?;
        if let Some(existing) = existing { 
            store.remove_episode(serie_id.to_string(), season, number).await?;
            self.add_deleted(library_id, RsDeleted::episode(serie_id.to_owned()), requesting_user).await?;
            self.send_episode(EpisodesMessage { library: library_id.to_string(), episodes: vec![EpisodeWithAction {action: ElementAction::Deleted, episode: existing.clone()}] });
            Ok(existing)
        } else {
            Err(Error::NotFound.into())
        }
	}

    pub async fn refresh_episodes(&self, library_id: &str, serie_id: &str, requesting_user: &ConnectedUser) -> RsResult<Vec<Episode>> {
        let ids = self.get_serie_ids(library_id, serie_id, requesting_user).await?;
        let all_episodes: Vec<Episode> = self.trakt.all_episodes(&ids).await?.into_iter().map(|mut e| {e.serie = serie_id.to_owned(); e}).collect();
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
        if MediasIds::is_id(serie_id) {
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
                let image_path = format!("cache/serie-{}-episode-{}x{}.webp", serie_id.replace(':', "-"), season, episode);

                if !local_provider.exists(&image_path).await {
                    let images = self.tmdb.episode_image(serie_ids, season, episode, &None).await?.into_kind(ImageType::Still).ok_or(crate::Error::NotFound)?;
                    let (_, mut writer) = local_provider.get_file_write_stream(&image_path).await?;
                    let image_reader = reqwest::get(images).await?;
                    let stream = image_reader.bytes_stream();
                    let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
                    let mut body_reader = StreamReader::new(body_with_io_error);
                    let resized = resize_image_reader(Box::pin(body_reader), ImageSize::Large.to_size(), image::ImageFormat::Avif, Some(60), false).await?;

                    writer.write_all(&resized).await?;
                }

                let source = local_provider.get_file(&image_path, None).await?;
                match source {
                    crate::plugins::sources::SourceRead::Stream(s) => Ok(s),
                    crate::plugins::sources::SourceRead::Request(_) => Err(crate::Error::GenericRedseatError),
                }
            }
        } else {
            if !self.has_library_image(library_id, &format!(".series/{}", serie_id), &format!("{}.{}", season, episode), None, requesting_user).await? {
                log_info(crate::tools::log::LogServiceType::Source, format!("Updating episode image: {}", serie_id));
                let r = self.refresh_episode_image(library_id, serie_id, season, episode, requesting_user).await;
                if let Err(r) = r {
                    println!("Error fetching episode image: {:?}", r);
                }
            }
            
            let image = self.library_image(library_id, &format!(".series/{}", serie_id), &format!("{}.{}", season, episode), None, size, requesting_user).await?;
            Ok(image)
        }
        
	}

    pub async fn get_episode_ids(&self, library_id: &str, serie_id: &str, season: u32, episode: u32, requesting_user: &ConnectedUser) -> RsResult<MediasIds> {
        let episode = self.get_episode(library_id, serie_id.to_string(), season, episode, requesting_user).await?;
        let ids: MediasIds = episode.into();
        Ok(ids)
    }

    /// download and update image
    pub async fn refresh_episode_image(&self, library_id: &str, serie_id: &str, season: &u32, episode: &u32, requesting_user: &ConnectedUser) -> RsResult<()> {
        let ids: MediasIds = self.get_episode_ids(library_id, serie_id, *season, *episode, requesting_user).await?;

        let reader = self.download_episode_image(&ids, season, episode, &None).await?;
        self.update_episode_image(library_id, serie_id, season, episode, reader, requesting_user).await?;
        Ok(())
    }
    pub async fn download_episode_image(&self, ids: &MediasIds, season: &u32, episode: &u32, lang: &Option<String>) -> crate::Result<AsyncReadPinBox> {
        let images = self.tmdb.episode_image(ids.clone(), season, episode, lang).await?.into_kind(ImageType::Still).ok_or(crate::Error::NotFound)?;
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
