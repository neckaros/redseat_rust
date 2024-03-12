


use std::{io::Read, pin::Pin};

use chrono::{Datelike, Utc};
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::{Value};
use tokio::io::{copy, AsyncRead};


use crate::{domain::{library::LibraryRole, media::{Media, MediaForAdd, MediaForInsert, MediaForUpdate, MediasMessage}, ElementAction}, plugins::sources::{AsyncReadPinBox, FileStreamResult}, routes::mw_range::RangeDefinition, tools::{file_tools::file_type_from_mime, image_tools::{ImageSize, ImageType}}};

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

    pub async fn get_media(&self, library_id: &str, media_id: String, requesting_user: &ConnectedUser) -> Result<Option<Media>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let mut media = store.get_media(&media_id).await?;
        if let Some(ref mut media) = media {
            if requesting_user.is_admin() {
                media.source = None;
            }
        }
		Ok(media)
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


    pub async fn add_media(&self, library_id: &str, new_media: MediaForAdd, notif: bool, requesting_user: &ConnectedUser) -> Result<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let media: MediaForInsert = new_media.into_insert();
		store.add_media(media.clone()).await?;
        let new_file = self.get_media(library_id, media.id, requesting_user).await?.ok_or(Error::NotFound)?;
        if notif { 
            self.send_media(MediasMessage { library: library_id.to_string(), action: ElementAction::Added, medias: vec![new_file.clone()] });
        }
		Ok(new_file)
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

    pub async fn update_media_image<T: AsyncRead>(&self, library_id: &str, media_id: &str, reader: T, requesting_user: &ConnectedUser) -> Result<()> {
        self.update_library_image(library_id, ".thumbs", media_id, &None, reader, requesting_user).await
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

    pub async fn add_library_file<T: AsyncRead>(&self, library_id: &str, filename: &str, infos: Option<MediaForUpdate>, reader: T, requesting_user: &ConnectedUser) -> Result<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;

        let m = self.source_for_library(&library_id).await?;


		let (source, writer) = m.get_file_write_stream(filename).await?;
        println!("Adding: {}", source);

		tokio::pin!(reader);
		tokio::pin!(writer);
		copy(&mut reader, &mut writer).await?;


        let mut infos = infos.unwrap_or_else(|| MediaForUpdate::default());
        let _ = m.fill_infos(&source, &mut infos).await;
        println!("new infos {:?}", infos);

        let mut new_file = MediaForAdd::default();
        new_file.name = filename.to_string();
        new_file.source = Some(source.to_string());
        new_file.mimetype = infos.mimetype.clone();
        new_file.created = Some(infos.created.unwrap_or_else(|| Utc::now().timestamp_millis() as u64));
        
        if let Some(ref mime) = new_file.mimetype {
            new_file.kind = file_type_from_mime(&mime);
        }

        println!("new file {:?}", new_file);

        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let id = nanoid!();
        store.add_media(MediaForInsert { id: id.clone(), media: new_file }).await?;
        
        store.update_media(&id, infos).await?;

        let media = store.get_media(&id).await?.ok_or(Error::NotFound)?;
        Ok(media)
	}

    pub async fn remove_library_file(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> Result<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_media_source(&media_id).await?;

        if let Some(existing) = existing {
            let m = self.source_for_library(&library_id).await?;
            m.remove(&existing.source).await?;
            store.remove_media(media_id.to_string()).await?;
        }
        self.remove_library_image(library_id, ".thumbs", media_id, &None, requesting_user).await?;


        Ok(())
	}

}
