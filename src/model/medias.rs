


use std::{io::Read, pin::Pin};

use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::{Value};
use tokio::{io::{AsyncRead}};


use crate::{domain::{library::LibraryRole, media::{Media, MediaForAdd, MediaForInsert, MediaForUpdate, MediasMessage}, ElementAction}, plugins::sources::{AsyncReadPinBox, FileStreamResult}, routes::mw_range::RangeDefinition, tools::image_tools::{ImageSize, ImageType}};

use super::{error::{Error, Result}, users::ConnectedUser, ModelController};



#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MediaQuery {
    pub after: Option<u64>
}

impl MediaQuery {
    pub fn new_empty() -> MediaQuery {
        MediaQuery { after: None }
    }
    pub fn from_after(after: u64) -> MediaQuery {
        MediaQuery { after: Some(after) }
    }
}

pub struct MediaSource {
    pub id: String,
    pub source: String
}

impl ModelController {

	pub async fn get_medias(&self, library_id: &str, query: MediaQuery, requesting_user: &ConnectedUser) -> Result<Vec<Media>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let people = store.get_medias(query).await?;
		Ok(people)
	}

    pub async fn get_media(&self, library_id: &str, tag_id: String, requesting_user: &ConnectedUser) -> Result<Option<Media>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let tag = store.get_media(&tag_id).await?;
		Ok(tag)
	}

    pub async fn update_media(&self, library_id: &str, media_id: String, update: MediaForUpdate, requesting_user: &ConnectedUser) -> Result<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		store.update_media(&media_id, update).await?;
        let person = store.get_media(&media_id).await?.ok_or(Error::NotFound)?;
        self.send_media(MediasMessage { library: library_id.to_string(), action: ElementAction::Updated, medias: vec![person.clone()] });
        Ok(person)
	}


	pub fn send_media(&self, message: MediasMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("tags", message);
			}
		});
	}


    pub async fn add_media(&self, library_id: &str, new_media: MediaForAdd, requesting_user: &ConnectedUser) -> Result<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let backup: MediaForInsert = new_media.into_insert();
		store.add_media(backup.clone()).await?;
        let new_person = self.get_media(library_id, backup.id, requesting_user).await?.ok_or(Error::NotFound)?;
        self.send_media(MediasMessage { library: library_id.to_string(), action: ElementAction::Added, medias: vec![new_person.clone()] });
		Ok(new_person)
	}


    pub async fn remove_media(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> Result<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_media(&media_id).await?;
        if let Some(existing) = existing { 
            store.remove_media(media_id.to_string()).await?;
            self.send_media(MediasMessage { library: library_id.to_string(), action: ElementAction::Removed, medias: vec![existing.clone()] });
            Ok(existing)
        } else {
            Err(Error::NotFound)
        }
	}


    
	pub async fn media_image(&self, library_id: &str, media_id: &str, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> Result<FileStreamResult<AsyncReadPinBox>> {
        let size = if let Some(s) = size {
                if s == ImageSize::Large {
                    None
                } else if s == ImageSize::Small {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            };
        self.library_image(library_id, ".thumbs", media_id, None, size, requesting_user).await
	}

    pub async fn update_media_image<T: AsyncRead>(&self, library_id: &str, media_id: &str, kind: &ImageType, reader: T, requesting_user: &ConnectedUser) -> Result<()> {
        self.update_library_image(library_id, ".thumbs", media_id, kind, reader, requesting_user).await
	}

    
	pub async fn library_file(&self, library_id: &str, media_id: &str, range: Option<RangeDefinition>, requesting_user: &ConnectedUser) -> Result<FileStreamResult<AsyncReadPinBox>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_media_source(&media_id).await?;

        if let Some(existing) = existing {
            let m = self.source_for_library(&library_id).await?;
            let reader_response = m.get_file_read_stream(&existing.source, range).await;


            Ok(reader_response?)
        } else {
            Err(Error::NotFound)
        }
	}

    
}
