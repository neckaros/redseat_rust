


use std::{io::{self, Read}, path::PathBuf, pin::Pin, str::FromStr};

use chrono::{Datelike, Utc};
use futures::TryStreamExt;
use http::header::CONTENT_TYPE;
use mime::{Mime, APPLICATION_OCTET_STREAM};
use mime_guess::get_mime_extensions_str;
use nanoid::nanoid;
use rs_plugin_common_interfaces::PluginType;
use serde::{Deserialize, Serialize};
use tokio::io::{copy, AsyncRead, AsyncReadExt};
use tokio_util::io::StreamReader;


use crate::{domain::{library::LibraryRole, media::{FileType, GroupMediaDownload, Media, MediaDownloadUrl, MediaForAdd, MediaForInsert, MediaForUpdate, MediaTagReference, MediasMessage}, ElementAction}, plugins::sources::{AsyncReadPinBox, FileStreamResult}, routes::mw_range::RangeDefinition, tools::{file_tools::{file_type_from_mime, get_extension_from_mime}, image_tools::{ImageSize, ImageType}, log::log_info, prediction::{predict_net, PredictionTagResult}}};

use super::{error::{Error, Result}, plugins::PluginQuery, users::ConnectedUser, ModelController};



#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct MediaQuery {
    pub after: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(rename = "type")]
    pub kind: Option<FileType>,
}

impl MediaQuery {
    pub fn new_empty() -> MediaQuery {
        MediaQuery { tags: vec![], ..Default::default() }
    }
    pub fn from_after(after: u64) -> MediaQuery {
        MediaQuery { after: Some(after), ..Default::default() }
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
            self.remove_library_file(&library_id, &media_id, &requesting_user).await?;
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

		tokio::pin!(reader);
		tokio::pin!(writer);
		copy(&mut reader, &mut writer).await?;


        let mut infos = infos.unwrap_or_else(|| MediaForUpdate::default());
        let _ = m.fill_infos(&source, &mut infos).await;

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

        self.generate_thumb(&library_id, &id, &requesting_user).await?;


        let media = store.get_media(&id).await?.ok_or(Error::NotFound)?;
        Ok(media)
	}


    pub async fn download_library_url(&self, library_id: &str, files: GroupMediaDownload<MediaDownloadUrl>, requesting_user: &ConnectedUser) -> Result<Vec<Media>> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;

        let m = self.source_for_library(&library_id).await?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let mut medias: Vec<Media> = vec![];
        //let infos = infos.unwrap_or_else(|| MediaForUpdate::default());
        for file in files.files {
            let mut infos = file.infos.unwrap_or_else(|| MediaForUpdate::default());
            let body = reqwest::get(file.url).await?;
            let headers = body.headers();

            let name = infos.name.clone();
            let mut filename = name.unwrap_or(nanoid!());
            if infos.mimetype.is_none() && headers.contains_key(CONTENT_TYPE) {
                infos.mimetype = headers.get(CONTENT_TYPE).and_then(|e| Some(e.to_str().and_then(|e| Ok(e.to_string())))).transpose()?;
            }

            if let Some(mimetype) = &infos.mimetype {
                let suffix = get_extension_from_mime(mimetype);
                filename = format!("{}.{}", filename, suffix);
   
                
            }


            let stream = body.bytes_stream();

            let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
            let body_reader = StreamReader::new(body_with_io_error);

            
            let (source, writer) = m.get_file_write_stream(&filename).await?;
            println!("Adding: {}", source);

            tokio::pin!(body_reader);
            tokio::pin!(writer);
            copy(&mut body_reader, &mut writer).await?;


            
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

            let id = nanoid!();
            store.add_media(MediaForInsert { id: id.clone(), media: new_file }).await?;
            
            store.update_media(&id, infos).await?;

            self.generate_thumb(&library_id, &id, &requesting_user).await?;
            let media = store.get_media(&id).await?.ok_or(Error::NotFound)?;
            medias.push(media)


        }
        Ok(medias)
	}

    pub async fn generate_thumb(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> Result<()> {
        let m = self.source_for_library(&library_id).await?; 
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let source = store.get_media_source(&media_id).await?.ok_or(Error::NotFound)?;
        let thumb = m.thumb(&source.source).await?;

        self.update_library_image(&library_id, ".thumbs", &media_id, &None, thumb.as_slice(), requesting_user).await?;

        Ok(())
    }


    pub async fn prediction(&self, library_id: &str, media_id: &str, insert_tags: bool, requesting_user: &ConnectedUser) -> crate::Result<Vec<PredictionTagResult>> {

        let plugins = self.get_plugins(PluginQuery { kind: Some(PluginType::ImageClassification), library: Some(library_id.to_string()), ..Default::default() }, requesting_user).await?;

        if plugins.len() > 0 {
            let mut all_predictions: Vec<PredictionTagResult> = vec![];
            let mut reader_response = self.media_image(&library_id, &media_id, None, &requesting_user).await?;

            for plugin in plugins {
                let mut buffer = Vec::new();
                reader_response.stream.read_to_end(&mut buffer).await?;
                let mut prediction = predict_net(PathBuf::from_str(&plugin.path).unwrap(), plugin.settings.bgr.unwrap_or(false), plugin.settings.normalize.unwrap_or(false), buffer)?;
                prediction.sort_by(|a, b| b.probability.partial_cmp(&a.probability).unwrap());
                if insert_tags {
                    for tag in &prediction {
                        let db_tag = self.get_ai_tag(&library_id, tag.tag.clone(), &requesting_user).await?;
                        self.update_media(&library_id, media_id.to_string(), MediaForUpdate { add_tags: Some(vec![MediaTagReference { id: db_tag.id, conf: Some(tag.probability as u16) }]), ..Default::default() }, &requesting_user).await?;
                    }
                }
                all_predictions.append(&mut prediction);
            }

            Ok(all_predictions)
        } else {
            Err(crate::Error::NoModelFound)
        }
    }
    
    pub async fn remove_library_file(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> Result<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_media_source(&media_id).await?;

        if let Some(existing) = existing {
            let m = self.source_for_library(&library_id).await?;
            let r = m.remove(&existing.source).await;
            if r.is_ok() {
				log_info(crate::tools::log::LogServiceType::Source, format!("Deleted file {}", existing.source));
			}
            store.remove_media(media_id.to_string()).await?;
        }
        self.remove_library_image(library_id, ".thumbs", media_id, &None, requesting_user).await?;


        Ok(())
	}

}
