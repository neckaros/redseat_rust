


use std::{io::Read, pin::Pin};

use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::{Value};
use tokio::{fs::File, io::{AsyncRead, BufReader}};


use crate::{domain::{library::LibraryRole, people::{PeopleMessage, Person}, serie::{Serie, SeriesMessage}, ElementAction}, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::image_tools::{ImageSize, ImageType}};

use super::{error::{Error, Result}, users::ConnectedUser, ModelController};


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SerieForAdd {
	pub name: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
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
	pub name: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
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

impl From<SerieForAdd> for SerieForInsert {
    fn from(new_serie: SerieForAdd) -> Self {
        SerieForInsert {
            id: nanoid!(),
            name: new_serie.name,
            kind: new_serie.kind,
            alt: new_serie.alt,
            params: new_serie.params,
            imdb: new_serie.imdb,
            slug: new_serie.slug,
            tmdb: new_serie.tmdb,
            trakt: new_serie.trakt,
            tvdb: new_serie.tvdb,
            otherids: new_serie.otherids,
            imdb_rating: new_serie.imdb_rating,
            imdb_votes: new_serie.imdb_votes,
            trakt_rating: new_serie.trakt_rating,
            trakt_votes: new_serie.trakt_votes,
            trailer: new_serie.trailer,
            year: new_serie.year
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SerieForUpdate {
	pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,

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
    pub trakt_rating: Option<u64>,
    pub trakt_votes: Option<f32>,

    pub trailer: Option<String>,

    pub year: Option<u16>,

    
    pub max_created: Option<u64>,
}



impl ModelController {

	pub async fn get_series(&self, library_id: &str, query: SerieQuery, requesting_user: &ConnectedUser) -> Result<Vec<Serie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let people = store.get_series(query).await?;
		Ok(people)
	}

    pub async fn get_serie(&self, library_id: &str, tag_id: String, requesting_user: &ConnectedUser) -> Result<Option<Serie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let tag = store.get_serie(&tag_id).await?;
		Ok(tag)
	}

    pub async fn update_serie(&self, library_id: &str, serie_id: String, update: SerieForUpdate, requesting_user: &ConnectedUser) -> Result<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		store.update_serie(&serie_id, update).await?;
        let person = store.get_serie(&serie_id).await?.ok_or(Error::NotFound)?;
        self.send_serie(SeriesMessage { library: library_id.to_string(), action: ElementAction::Updated, series: vec![person.clone()] });
        Ok(person)
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
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
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


    
	pub async fn serie_image(&self, library_id: &str, serie_id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> Result<FileStreamResult<AsyncReadPinBox>> {
        self.library_image(library_id, ".series", serie_id, kind.or(Some(ImageType::Poster)), size, requesting_user).await
	}

    pub async fn update_serie_image<T: AsyncRead>(&self, library_id: &str, serie_id: &str, kind: &ImageType, reader: T, requesting_user: &ConnectedUser) -> Result<()> {
        self.update_library_image(library_id, ".series", serie_id, kind, reader, requesting_user).await
	}
    
}
