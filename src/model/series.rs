


use std::{io::{self, Read}, pin::Pin};

use futures::TryStreamExt;
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{fs::File, io::{AsyncRead, AsyncWriteExt, BufReader}};
use tokio_util::io::StreamReader;


use crate::{domain::{library::LibraryRole, people::{PeopleMessage, Person}, serie::{Serie, SeriesMessage}, ElementAction, MediasIds}, error::RsResult, plugins::{medias::imdb::ImdbContext, sources::{path_provider::PathProvider, AsyncReadPinBox, FileStreamResult, Source}}, server::get_server_folder_path_array, tools::{image_tools::{resize_image_reader, ImageSize, ImageType}, log::log_info}};

use super::{error::{Error, Result}, users::ConnectedUser, ModelController};


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SerieForAdd {
	pub name: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub status: Option<String>,
    pub alt: Option<Vec<String>>,
    pub params: Option<Value>,
    pub imdb: Option<String>,
    pub slug: Option<String>,
    pub tmdb: Option<u64>,
    pub trakt: Option<u64>,
    pub tvdb: Option<u64>,
    pub otherids: Option<String>,
    
    pub imdb_rating: Option<f32>,
    pub imdb_votes: Option<u64>,
    pub trakt_rating: Option<u64>,
    pub trakt_votes: Option<f32>,

    pub trailer: Option<String>,


    pub year: Option<u16>,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SerieForInsert {
    pub id: String,
    pub serie: SerieForAdd,
}



impl From<SerieForAdd> for SerieForInsert {
    fn from(new_serie: SerieForAdd) -> Self {
        SerieForInsert {
            id: nanoid!(),
            serie: new_serie
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SerieQuery {
    pub after: Option<u64>
}

impl SerieQuery {
    pub fn new_empty() -> SerieQuery {
        SerieQuery { after: None }
    }
    pub fn from_after(after: u64) -> SerieQuery {
        SerieQuery { after: Some(after) }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SerieForUpdate {
	pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub status: Option<String>,
    pub alt: Option<Vec<String>>,
    pub add_alts: Option<Vec<String>>,
    pub remove_alts: Option<Vec<String>>,

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

    pub trailer: Option<String>,

    pub year: Option<u16>,
    pub max_created: Option<u64>,
}

impl SerieForUpdate {
    pub fn has_update(&self) -> bool {
        self.name.is_some() || self.kind.is_some() || self.status.is_some() || self.alt.is_some() || self.add_alts.is_some() || self.remove_alts.is_some()
        || self.params.is_some() || self.imdb.is_some() || self.slug.is_some() || self.tmdb.is_some() || self.trakt.is_some() || self.tvdb.is_some() || self.otherids.is_some()
        || self.imdb_rating.is_some() || self.imdb_votes.is_some() || self.trakt_rating.is_some() || self.trakt_votes.is_some()
        || self.trailer.is_some() || self.year.is_some() || self.max_created.is_some()
    } 
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExternalSerieImages {
    pub backdrop: Option<String>,
    pub logo: Option<String>,
    pub poster: Option<String>,
    pub still: Option<String>,
}

impl ExternalSerieImages {
    pub fn into_kind(self, kind: ImageType) -> Option<String> {
        match kind {
            ImageType::Poster => self.poster,
            ImageType::Background => self.backdrop,
            ImageType::Still => self.still,
            ImageType::Card => None,
            ImageType::ClearLogo => self.logo,
            ImageType::ClearArt => None,
            ImageType::Custom(_) => None,
        }
    }
}

impl Serie {
    pub async fn fill_imdb_ratings(&mut self, imdb_context: &ImdbContext) {
        if let Some(imdb) = &self.imdb {
            let rating = imdb_context.get_rating(&imdb).await.unwrap_or(None);
            if let Some(rating) = rating {
                self.imdb_rating = Some(rating.0);
                self.imdb_votes = Some(rating.1);
            }
        }
    } 
}


impl ModelController {

	pub async fn get_series(&self, library_id: &str, query: SerieQuery, requesting_user: &ConnectedUser) -> Result<Vec<Serie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let people = store.get_series(query).await?;
		Ok(people)
	}

    pub async fn get_serie(&self, library_id: &str, serie_id: String, requesting_user: &ConnectedUser) -> Result<Option<Serie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;

        if MediasIds::is_id(&serie_id) {
            let id: MediasIds = serie_id.try_into().map_err(|_| Error::NotFound)?;
            let serie = store.get_serie_by_external_id(id.clone()).await?;
            if let Some(serie) = serie {
                Ok(Some(serie))
            } else {
                let mut trakt_show = self.trakt.get_serie(&id).await.map_err(|_| Error::NotFound)?;
                trakt_show.fill_imdb_ratings(&self.imdb).await;
                Ok(Some(trakt_show))
            }
        } else {
            let serie = store.get_serie(&serie_id).await?;
            Ok(serie)
        }
	}

    pub async fn trending_shows(&self)  -> RsResult<Vec<Serie>> {
        self.trakt.trending_shows().await
    }




    pub async fn update_serie(&self, library_id: &str, serie_id: String, update: SerieForUpdate, requesting_user: &ConnectedUser) -> Result<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        if MediasIds::is_id(&serie_id) {
            return Err(Error::InvalidIdForAction("udpate".to_string(), serie_id))
        }
        if update.has_update() {
            let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
            store.update_serie(&serie_id, update).await?;
            let person = store.get_serie(&serie_id).await?.ok_or(Error::NotFound)?;
            self.send_serie(SeriesMessage { library: library_id.to_string(), action: ElementAction::Updated, series: vec![person.clone()] });
            Ok(person)
        } else {
            let serie = self.get_serie(library_id, serie_id, requesting_user).await?.ok_or(Error::NotFound)?;
            Ok(serie)
        }  
	}


	pub fn send_serie(&self, message: SeriesMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("tags", message);
			}
		});
	}


    pub async fn add_serie(&self, library_id: &str, new_serie: SerieForAdd, requesting_user: &ConnectedUser) -> Result<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let backup: SerieForInsert = new_serie.into();
		store.add_serie(backup.clone()).await?;
        let new_person = self.get_serie(library_id, backup.id, requesting_user).await?.ok_or(Error::NotFound)?;
        self.send_serie(SeriesMessage { library: library_id.to_string(), action: ElementAction::Added, series: vec![new_person.clone()] });
		Ok(new_person)
	}


    pub async fn remove_serie(&self, library_id: &str, serie_id: &str, requesting_user: &ConnectedUser) -> Result<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if MediasIds::is_id(&serie_id) {
            return Err(Error::InvalidIdForAction("remove".to_string(), serie_id.to_string()))
        }
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_serie(&serie_id).await?;
        if let Some(existing) = existing { 
            store.remove_serie(serie_id.to_string()).await?;
            self.send_serie(SeriesMessage { library: library_id.to_string(), action: ElementAction::Removed, series: vec![existing.clone()] });
            Ok(existing)
        } else {
            Err(Error::NotFound)
        }
	}

    pub async fn refresh_serie(&self, library_id: &str, serie_id: &str, requesting_user: &ConnectedUser) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let ids = self.get_serie_ids(library_id, serie_id, requesting_user).await?;
        let serie = self.get_serie(library_id, serie_id.to_string(), requesting_user).await?.ok_or(Error::NotFound)?;
        let new_serie = self.trakt.get_serie(&ids).await?;
        let mut updates = SerieForUpdate {..Default::default()};

        if serie.status != new_serie.status {
            updates.status = new_serie.status;
        }
        if serie.trakt_rating != new_serie.trakt_rating {
            updates.trakt_rating = new_serie.trakt_rating;
        }
        if serie.trakt_votes != new_serie.trakt_votes {
            updates.trakt_votes = new_serie.trakt_votes;
        }
        if serie.trailer != new_serie.trailer {
            updates.trailer = new_serie.trailer;
        }
        if serie.imdb != new_serie.imdb {
            updates.imdb = new_serie.imdb;
        }
        if serie.tmdb != new_serie.tmdb {
            updates.tmdb = new_serie.tmdb;
        }

        let new_serie = self.update_serie(library_id, serie_id.to_string(), updates, requesting_user).await?;
        Ok(new_serie)        
	}

    
	pub async fn serie_image(&self, library_id: &str, serie_id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> crate::Result<FileStreamResult<AsyncReadPinBox>> {
        let kind = kind.unwrap_or(ImageType::Poster);
        if MediasIds::is_id(&serie_id) {
            let mut serie_ids: MediasIds = serie_id.to_string().try_into()?;

            let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
            let existing_serie = store.get_serie_by_external_id(serie_ids.clone()).await?;
            if let Some(existing_serie) = existing_serie {
                let image = self.library_image(library_id, ".series", &existing_serie.id, Some(kind), size, requesting_user).await?;
                Ok(image)
            } else {

                let local_provider = self.library_source_for_library(library_id).await?;
                
                if serie_ids.tmdb.is_none() {
                    let serie = self.trakt.get_serie(&serie_ids).await?;
                    serie_ids = serie.into();
                }
                let image_path = format!("cache/serie-{}-{}.webp", serie_id.replace(":", "-"), kind);

                if !local_provider.exists(&image_path).await {
                    let (_, mut writer) = local_provider.get_file_write_stream(&image_path).await?;
                    let images = self.tmdb.serie_image(serie_ids).await?.into_kind(kind).ok_or(crate::Error::NotFound)?;
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
            if !self.has_library_image(library_id, ".series", serie_id, Some(kind.clone()), requesting_user).await? {
                log_info(crate::tools::log::LogServiceType::Source, format!("Updating serie image: {}", serie_id));
                self.refresh_serie_image(library_id, serie_id, &kind, requesting_user).await?;
            }
            
            let image = self.library_image(library_id, ".series", serie_id, Some(kind), size, requesting_user).await?;
            Ok(image)
        }
	}

    /// download and update image
    pub async fn refresh_serie_image(&self, library_id: &str, serie_id: &str, kind: &ImageType, requesting_user: &ConnectedUser) -> RsResult<()> {
        let serie = self.get_serie(library_id, serie_id.to_string(), requesting_user).await?.ok_or(Error::NotFound)?;
        let ids: MediasIds = serie.into();
        let reader = self.download_serie_image(&ids, kind).await?;
        self.update_serie_image(library_id, serie_id, kind, reader, requesting_user).await?;
        Ok(())
	}
    pub async fn download_serie_image(&self, ids: &MediasIds, kind: &ImageType) -> crate::Result<AsyncReadPinBox> {
        let images = self.tmdb.serie_image(ids.clone()).await?.into_kind(kind.clone()).ok_or(crate::Error::NotFound)?;
        let image_reader = reqwest::get(images).await?;
        let stream = image_reader.bytes_stream();
        let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
        let mut body_reader = StreamReader::new(body_with_io_error);
        Ok(Box::pin(body_reader))
    }

    pub async fn update_serie_image<T: AsyncRead>(&self, library_id: &str, serie_id: &str, kind: &ImageType, reader: T, requesting_user: &ConnectedUser) -> Result<()> {
        if MediasIds::is_id(&serie_id) {
            return Err(Error::InvalidIdForAction("udpate image".to_string(), serie_id.to_string()))
        }
        self.update_library_image(library_id, ".series", serie_id, &Some(kind.clone()), reader, requesting_user).await
	}
    
}
