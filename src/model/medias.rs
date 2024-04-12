


use std::{io::{self, Cursor, Read}, path::PathBuf, pin::Pin, result, str::FromStr};

use chrono::{Datelike, Utc};
use futures::TryStreamExt;
use http::header::CONTENT_TYPE;
use mime::{Mime, APPLICATION_OCTET_STREAM};
use mime_guess::get_mime_extensions_str;
use nanoid::nanoid;
use query_external_ip::SourceError;
use rs_plugin_common_interfaces::{request::{RsRequest, RsRequestStatus}, PluginType};
use serde::{Deserialize, Serialize};
use strum_macros::EnumString;
use tokio::{io::{copy, AsyncRead, AsyncReadExt}, sync::mpsc};
use tokio_util::io::StreamReader;
use crate::{domain::{media::{RsGpsPosition, DEFAULT_MIME}, MediaElement}, model::store::sql::SqlOrder, tools::image_tools::convert_image_reader};

use crate::{domain::{library::LibraryRole, media::{FileType, GroupMediaDownload, Media, MediaDownloadUrl, MediaForAdd, MediaForInsert, MediaForUpdate, MediaItemReference, MediaWithAction, MediasMessage, ProgressMessage}, progress::{RsProgress, RsProgressType}, ElementAction}, error::RsResult, plugins::{get_plugin_fodler, sources::{async_reader_progress::ProgressReader, error::SourcesError, AsyncReadPinBox, FileStreamResult, SourceRead}}, routes::mw_range::RangeDefinition, server::get_server_port, tools::{auth::{sign_local, ClaimsLocal}, file_tools::{file_type_from_mime, get_extension_from_mime}, image_tools::{self, resize_image, resize_image_reader, ImageSize, ImageType}, log::{log_error, log_info, LogServiceType}, prediction::{predict_net, preload_model, PredictionTagResult}, video_tools::{self, probe_video, VideoTime}}};

use super::{error::{Error, Result}, plugins::PluginQuery, store, users::ConnectedUser, ModelController};

pub const CRYPTO_HEADER_SIZE: u64 = 16 + 4 + 4 + 32 + 256;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct MediaQuery {
    
    #[serde(default)]
    pub sort: RsSort,
    #[serde(default)]
    pub order: SqlOrder,

    pub before: Option<u64>,
    pub after: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub people: Vec<String>,
    #[serde(default)]
    pub series: Vec<String>,
    pub limit: Option<usize>,
    #[serde(default)]
    pub types: Vec<FileType>,
    
    pub page_key: Option<u64>,

    pub filter: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct MediaFileQuery {
    #[serde(default)]
    pub unsupported_mime: Vec<String>,
    
    #[serde(default)]
    pub raw: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display,EnumString, Default)]
#[strum(serialize_all = "camelCase")]
#[serde(rename_all = "camelCase")]
pub enum RsSort {
    #[default]
    Modified,
    Added,
    Created,
    Rating,
    Name,
    Size,
    #[strum(default)]
    Custom(String)
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct  RsSortOrder {
    pub order: SqlOrder,
    pub sort: RsSort
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
    pub source: String,
    pub kind: FileType,
    pub thumb_size: Option<u64>,
    pub size: Option<u64>,
    
    pub mime: String,
}
impl TryFrom<Media> for MediaSource {
    type Error = crate::model::error::Error;
    fn try_from(value: Media) -> std::prelude::v1::Result<Self, Self::Error> {
        let source = value.source.ok_or(crate::model::error::Error::NoSourceForMedia)?;
        Ok(MediaSource {
            id: value.id,
            source,
            kind: value.kind,
            thumb_size: value.thumbsize,
            size: value.size,
            mime: value.mimetype.unwrap_or(DEFAULT_MIME.to_owned()),
        })
    }
}

impl ModelController {

	pub async fn get_medias(&self, library_id: &str, query: MediaQuery, requesting_user: &ConnectedUser) -> Result<Vec<Media>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let people = store.get_medias(query).await?;
		Ok(people)
	}

    pub async fn get_locs(&self, library_id: &str, precision: Option<u32>, requesting_user: &ConnectedUser) -> Result<Vec<RsGpsPosition>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let locs = store.get_medias_locs(precision.unwrap_or(2)).await?;
		Ok(locs)
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
        let media = store.get_media(&media_id).await?.ok_or(Error::NotFound)?;
        self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: media.clone(), action: ElementAction::Updated}] });
        Ok(media)
	}


	pub fn send_media(&self, message: MediasMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("medias", message);
			}
		});
	}

	pub fn send_progress(&self, message: ProgressMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("medias_progress", message);
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
            self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: new_file.clone(), action: ElementAction::Added}] });
        }
		Ok(new_file)
	}

    pub async fn remove_media(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> Result<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_media(&media_id).await?;
        if let Some(existing) = existing { 
            self.remove_library_file(&library_id, &media_id, &requesting_user).await?;
            self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: existing.clone(), action: ElementAction::Removed}] });
            Ok(existing)
        } else {
            Err(Error::NotFound)
        }
	}

	pub async fn media_image(&self, library_id: &str, media_id: &str, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> RsResult<FileStreamResult<AsyncReadPinBox>> {

		if self.cache_get_library_crypt(library_id).await {
			let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
			let media_source = store.get_media_source(media_id).await?.ok_or(Error::NotFound)?;
			//headerSize(), end: headerSize() + fileInfo.thumbsize - 1 }
            let range = RangeDefinition { start: Some(CRYPTO_HEADER_SIZE), end: Some(CRYPTO_HEADER_SIZE + media_source.thumb_size.unwrap_or(0)) };
			let source = self.source_for_library(library_id).await?;
			let reader = source.get_file(&media_source.source, Some(range)).await?;
			if let SourceRead::Stream(mut reader) = reader {
                reader.range = None;
                reader.size = media_source.thumb_size;
				return Ok(reader);
			} else {
				return Err(Error::NotFound.into())
			}
            
		}


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

        let result = self.library_image(library_id, ".thumbs", media_id, None, size.clone(), requesting_user).await;
        if let Err(error) = result {
            if let crate::Error::Source(s) = &error {
                if let SourcesError::NotFound(_) = s {
                    println!("regen");
                    self.generate_thumb(library_id, media_id, requesting_user).await.map_err(|_| Error::NotFound)?;
                    println!("regened");
                    let result = self.library_image(library_id, ".thumbs", media_id, None, size, requesting_user).await;
                    result
                } else {
                    Err(error)
                }
            } else {
                Err(error)
            }
        } else {
            result
        }
        
	}

    pub async fn update_media_image<T: AsyncRead>(&self, library_id: &str, media_id: &str, reader: T, requesting_user: &ConnectedUser) -> Result<()> {
        self.update_library_image(library_id, ".thumbs", media_id, &None, reader, requesting_user).await
	}

    
	pub async fn library_file(&self, library_id: &str, media_id: &str, mut range: Option<RangeDefinition>, query: MediaFileQuery, requesting_user: &ConnectedUser) -> RsResult<SourceRead> {
        requesting_user.check_file_role(library_id, media_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_media_source(&media_id).await?;
        let crypted = self.cache_get_library(library_id).await.and_then(|l| l.crypt).unwrap_or(false);
        if let Some(existing) = existing {
            let m = self.source_for_library(&library_id).await?;
            if crypted && query.raw == false {
                range = Some(RangeDefinition { start: Some(CRYPTO_HEADER_SIZE + existing.thumb_size.unwrap_or(0)), end: None })
            }
            let mut reader_response = m.get_file(&existing.source, range.clone()).await?;

            if crypted {
                if let SourceRead::Stream(reader) = &mut reader_response {
                    reader.range = None;
                    reader.size = existing.size;
                }
            }

            if query.unsupported_mime.len() > 0 {
                if existing.kind == FileType::Photo && query.unsupported_mime.contains(&existing.mime) || query.unsupported_mime.contains(&"all".to_owned()) {
                    let mut data = reader_response.into_reader(library_id, range, None, Some((self.clone(), &requesting_user))).await?; 
                    let resized = convert_image_reader(&mut data.stream, "jpg", Some(80)).await?;
                    let len = resized.len();
                    let resized = Cursor::new(resized);
                    Ok(SourceRead::Stream(FileStreamResult {
                        stream: Box::pin(resized),
                        size: Some(len as u64),
                        accept_range: false,
                        range: None,
                        mime: Some("image/webp".to_owned()),
                        name: Some("converted.webp".to_owned()),
                        cleanup: None,
                    }))
                } else {
                    Ok(reader_response)
                }
            } else {
                Ok(reader_response)
            }
        } else {
            Err(Error::NotFound.into())
        }
	}
    pub fn process_media_spawn(&self, library_id: String, media_id: String, requesting_user: ConnectedUser){
        let mc = self.clone();
        tokio::spawn(async move {
            let r = mc.process_media(&library_id, &media_id, &requesting_user).await;
            if let Err(error) = r {
                log_error(crate::tools::log::LogServiceType::Source, format!("Unable to process media {} for predictions: {:?}", media_id, error));
            }
        });
    }
    pub async fn process_media(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> RsResult<()>{
        self.cache_check_library_notcrypt(library_id).await?;

        let existing = self.get_media(library_id, media_id.to_owned(), requesting_user).await?.ok_or(Error::NotFound)?;

        if existing.kind == FileType::Video {
            let r = self.update_video_infos(library_id, media_id, requesting_user).await;
            if let Err(r) = r {
                log_error(LogServiceType::Source, format!("unable to get video infos for {}: {:?}", media_id, r));
            }
        } else if existing.kind == FileType::Photo {
            let r = self.update_photo_infos(library_id, media_id, requesting_user).await;
            if let Err(r) = r {
                log_error(LogServiceType::Source, format!("unable to get photos infos for {}: {:?}", media_id, r));
            }
        }



        let _ = self.prediction(library_id, media_id, true, requesting_user).await;
        Ok(())
    }


    
    pub async fn add_library_file<T: AsyncRead>(&self, library_id: &str, filename: &str, infos: Option<MediaForUpdate>, reader: T, requesting_user: &ConnectedUser) -> Result<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let crypted = self.cache_get_library_crypt(library_id).await;
        let m = self.source_for_library(&library_id).await?;


		let (source, writer) = m.get_file_write_stream(filename).await?;

		tokio::pin!(reader);
		tokio::pin!(writer);
		copy(&mut reader, &mut writer).await?;


        let mut infos = infos.unwrap_or_else(|| MediaForUpdate::default());
        if !crypted {
            let _ = m.fill_infos(&source, &mut infos).await;
        }

        let mut new_file = MediaForAdd::default();
        new_file.name = filename.to_string();
        new_file.source = Some(source.to_string());
        new_file.mimetype = infos.mimetype.clone();
        new_file.created = Some(infos.created.unwrap_or_else(|| Utc::now().timestamp_millis() as u64));
        
        if let Some(ref mime) = new_file.mimetype {
            new_file.kind = file_type_from_mime(&mime);
        }

        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let id = nanoid!();
        store.add_media(MediaForInsert { id: id.clone(), media: new_file }).await?;
        
        store.update_media(&id, infos).await?;

        if !crypted {
            let _ = self.generate_thumb(&library_id, &id, &requesting_user).await;
            self.process_media_spawn(library_id.to_string(), id.clone(), requesting_user.clone());
        }


        let media = store.get_media(&id).await?.ok_or(Error::NotFound)?;
        self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: media.clone(), action: ElementAction::Added}] });

        Ok(media)
	}


    pub async fn download_library_url(&self, library_id: &str, files: GroupMediaDownload<MediaDownloadUrl>, requesting_user: &ConnectedUser) -> RsResult<Vec<Media>> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        self.cache_check_library_notcrypt(library_id).await?;

        let m = self.source_for_library(&library_id).await?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let mut medias: Vec<Media> = vec![];
        let origin = if let Some(origin) = &files.origin_url {
            self.exec_parse(Some(library_id.to_owned()), origin.to_owned(), &requesting_user).await.ok()
        } else {
            None
        };

        let requests: Vec<RsRequest> = files.into(); 
    
        //let infos = infos.unwrap_or_else(|| MediaForUpdate::default());
        for mut request in requests {
            let upload_id = request.upload_id.clone().unwrap_or_else(|| nanoid!());

            self.plugin_manager.fill_infos(&mut request).await;
            let mut infos: MediaForUpdate = request.clone().into();
            infos.origin = origin.clone();
            
            if let Some(origin) = &origin {
                let existing = store.get_media_by_origin(origin.clone()).await;
                if let Some(existing) = existing {
                    return Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Media(existing)).into())
                }
            }

            //Progress
            let lib_progress = library_id.to_string();
            let mc_progress = self.clone();
            let name_progress = infos.name.clone().unwrap_or(upload_id.clone());
            let (tx_progress, mut rx_progress) = mpsc::channel::<RsProgress>(100);
            let progress_id = upload_id.clone();
            tokio::spawn(async move {
                let mut last_send = 0;
                let mut last_type: Option<RsProgressType> = None;
                
                while let Some(mut progress) = rx_progress.recv().await {
                    let current = progress.current.unwrap_or(1);
                    if last_send == 0 || current < last_send || current - last_send  > 1000000 || Some(&progress.kind) != last_type.as_ref() {
                        last_type = Some(progress.kind.clone());
                        last_send = current.clone();
                        progress.id = progress_id.clone();
                        let message = ProgressMessage {
                            library: lib_progress.clone(),
                            name: name_progress.clone(),
                            progress,
                        };
                        mc_progress.send_progress(message);
                    }
                }
            });



            let reader = SourceRead::Request(request).into_reader(library_id, None, 
                Some(tx_progress.clone()), Some((self.clone(), &ConnectedUser::ServerAdmin))).await?;

            let name = infos.name.clone();
            let mut filename = name.or_else(|| reader.name).unwrap_or(nanoid!());
            if infos.mimetype.is_none() {
                infos.mimetype = reader.mime;
            }

            if !filename.contains(".") || filename.split(".").last().unwrap_or("").len() > 5 {
                if let Some(mimetype) = &infos.mimetype {
                    let suffix = get_extension_from_mime(mimetype);
                    filename = format!("{}.{}", filename, suffix);
                }
            }
            
            let (source, writer) = m.get_file_write_stream(&filename).await?;
            let mut progress_reader = ProgressReader::new(reader.stream, RsProgress { id: upload_id.clone(), total: reader.size, current: Some(0), kind: RsProgressType::Transfert }, tx_progress.clone());
            tokio::pin!(writer);

            copy(&mut progress_reader, &mut writer).await?;

            

            
            let _ = m.fill_infos(&source, &mut infos).await;

            if let Some(hash) = &infos.md5 {
                let existing = store.get_media_by_hash(hash.to_owned()).await;
                if let Some(existing) = existing {
                    m.remove(&source).await?;
                    return Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Media(existing)).into())
                }
            }

            let mut new_file = MediaForAdd::default();
            new_file.name = filename.to_string();
            new_file.source = Some(source.to_string());
            new_file.mimetype = infos.mimetype.clone();
            new_file.created = Some(infos.created.unwrap_or_else(|| Utc::now().timestamp_millis() as u64));

            let final_progress = tx_progress.send(RsProgress { id: upload_id.clone(), total: infos.size, current: infos.size, kind: RsProgressType::Finished }).await;
            if let Err(error) = final_progress {
                log_error(LogServiceType::Source, format!("Unable to send final progress message: {:?}", error));
            }
            
            if let Some(ref mime) = new_file.mimetype {
                new_file.kind = file_type_from_mime(&mime);
            }

            let id = nanoid!();
            store.add_media(MediaForInsert { id: id.clone(), media: new_file }).await?;
            
            store.update_media(&id, infos).await?;

            let r = self.generate_thumb(&library_id, &id, &ConnectedUser::ServerAdmin).await;
            self.process_media_spawn(library_id.to_string(), id.clone(), ConnectedUser::ServerAdmin);
            
            if let Err(r) = r {
                log_error(crate::tools::log::LogServiceType::Source, format!("Unable to generate thumb {:#}", r));
            }
            let media = store.get_media(&id).await?.ok_or(Error::NotFound)?;
            self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: media.clone(), action: ElementAction::Added}] });
            
            medias.push(media)


        }
        Ok(medias)
	}
    
    pub async fn medias_add_request(&self, library_id: &str, request: RsRequest, additional_infos: Option<MediaForUpdate>, requesting_user: &ConnectedUser) -> RsResult<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
     
        let processed_request = if request.status == RsRequestStatus::Unprocessed {
            self.exec_request(request.clone(), Some(library_id.to_string()), true, None, &requesting_user).await?
        } else {
            SourceRead::Request(request.clone())
        };

        let infos: MediaForUpdate = request.clone().into();

        let final_infos: MediaForUpdate = (&processed_request).into();


     
        let mut new_file = MediaForAdd::default();
        new_file.name = final_infos.name.or_else(|| infos.name).unwrap_or(nanoid!());
        new_file.source = Some(request.url);
        new_file.mimetype = final_infos.mimetype.or_else(|| infos.mimetype);
        new_file.size = final_infos.size.or_else(|| infos.size);
        new_file.created = Some(infos.created.unwrap_or_else(|| Utc::now().timestamp_millis() as u64));
        
        if let Some(ref mime) = new_file.mimetype {
            new_file.kind = file_type_from_mime(&mime);
        }

        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let id = nanoid!();
        store.add_media(MediaForInsert { id: id.clone(), media: new_file }).await?;
        
        if let Some(update) = additional_infos {
            self.update_media(&library_id, id.clone(), update, &requesting_user).await?;
        }

        let media = store.get_media(&id).await?.ok_or(Error::NotFound)?;
        self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: media.clone(), action: ElementAction::Added}] });

        Ok(media)
	}


    pub async fn generate_thumb(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> crate::error::Result<()> {
        self.cache_check_library_notcrypt(library_id).await?;

        let m = self.source_for_library(&library_id).await?; 
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let media = store.get_media(&media_id).await?.ok_or(Error::NotFound)?;
        
        let thumb = match media.kind {
            FileType::Photo => { 
                let media_source: MediaSource = media.try_into()?;
                println!("Photo {}", &media_source.source);
                let th = m.thumb(&media_source.source).await?;
                Ok(th)
            },
            FileType::Video => { 
                let th = self.get_video_thumb(library_id, media_id, VideoTime::Percent(5), requesting_user).await?;
                Ok(th)
            },
            _ => Err(crate::model::error::Error::UnsupportedTypeForThumb),
        }?;
        
        self.update_library_image(&library_id, ".thumbs", &media_id, &None, thumb.as_slice(), requesting_user).await?;

        Ok(())
    }

    pub async fn get_video_thumb(&self, library_id: &str, media_id: &str, time: VideoTime, requesting_user: &ConnectedUser) -> crate::error::Result<Vec<u8>> {
        requesting_user.check_file_role(library_id, media_id, LibraryRole::Read)?;

        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let source = store.get_media_source(&media_id).await?.ok_or(Error::NotFound)?;

        let m = self.source_for_library(&library_id).await?; 
        let local_path = m.local_path(&source.source);
        let uri = if let Some(local_path) = local_path {
            local_path.to_str().unwrap().to_string()
        } else {
            ModelController::get_temporary_local_read_url(library_id, media_id).await?
        };
        let thumb = video_tools::thumb_video(&uri, time).await?;
        let mut cursor = std::io::Cursor::new(thumb);
        let thumb = resize_image_reader(&mut cursor, 512).await?;
        Ok(thumb)
    }

    pub async  fn get_temporary_local_read_url(library_id: &str, media_id: &str) -> Result<String> {
        let exp = ClaimsLocal::generate_seconds(240);
        let claims = ClaimsLocal {
            cr: "service::get_video_thumb".to_string(),
            kind: crate::tools::auth::ClaimsLocalType::File(library_id.to_string(), media_id.to_string()),
            exp,
        };
    
        let local_port = get_server_port().await;
        let token = sign_local(claims).await.map_err(|_| Error::UnableToSignShareToken)?;
        let uri = format!("http://localhost:{}/libraries/{}/medias/{}?sharetoken={}", local_port, library_id, media_id, token);
        Ok(uri)
    }


    pub async  fn get_file_share_token(&self, library_id: &str, media_id: &str, delay_in_seconds: u64, requesting_user: &ConnectedUser) -> Result<String> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let exp = ClaimsLocal::generate_seconds(delay_in_seconds);
        let claims = ClaimsLocal {
            cr: "service::share_media".to_string(),
            kind: crate::tools::auth::ClaimsLocalType::File(library_id.to_string(), media_id.to_string()),
            exp,
        };
        let token = sign_local(claims).await.map_err(|_| Error::UnableToSignShareToken)?;
        Ok(token)
    }

    pub async fn prediction(&self, library_id: &str, media_id: &str, insert_tags: bool, requesting_user: &ConnectedUser) -> crate::Result<Vec<PredictionTagResult>> {

        let plugins = self.get_plugins(PluginQuery { kind: Some(PluginType::ImageClassification), library: Some(library_id.to_string()), ..Default::default() }, requesting_user).await?;
        if plugins.len() > 0 {
            let mut all_predictions: Vec<PredictionTagResult> = vec![];
            let media = self.get_media(library_id, media_id.to_string(), requesting_user).await?.ok_or(Error::NotFound)?;

            let mut reader_response = self.media_image(&library_id, &media_id, None, &requesting_user).await?;
            let mut buffer = Vec::new();
            reader_response.stream.read_to_end(&mut buffer).await?;

            let mut images = vec![buffer];
            if media.kind == FileType::Video {
                let percents = vec![15, 30, 45, 60, 75, 95];
                for percent in percents {
                    let thumb = self.get_video_thumb(library_id, media_id, VideoTime::Percent(percent), requesting_user).await?;
                    images.push(thumb);
                }
            }
            
            for plugin in plugins.clone() {
                let mut path = get_plugin_fodler().await?;
                    path.push(&plugin.path);
                let model: ort::Session = preload_model(&path)?;
                for buffer in &images {
                    
                    let mut prediction = predict_net(path.clone(), plugin.settings.bgr.unwrap_or(false), plugin.settings.normalize.unwrap_or(false), buffer.clone(), Some(&model))?;
                    prediction.sort_by(|a, b| b.probability.partial_cmp(&a.probability).unwrap());
                    if insert_tags {
                        for tag in &prediction {
                            let db_tag = self.get_ai_tag(&library_id, tag.tag.clone(), &requesting_user).await?;
                            self.update_media(&library_id, media_id.to_string(), MediaForUpdate { add_tags: Some(vec![MediaItemReference { id: db_tag.id, conf: Some(tag.probability as u16) }]), ..Default::default() }, &requesting_user).await?;
                        }
                    }
                    all_predictions.append(&mut prediction);
                }
            }

            Ok(all_predictions)
        } else {
            Err(crate::Error::NoModelFound)
        }
    }
    
    pub async fn update_video_infos(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> crate::Result<()> {
        requesting_user.check_file_role(library_id, media_id, LibraryRole::Read)?;

        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let source = store.get_media_source(&media_id).await?.ok_or(Error::NotFound)?;

        let m = self.source_for_library(&library_id).await?; 
        let local_path = m.local_path(&source.source);
        let uri = if let Some(local_path) = local_path {
            local_path.to_str().unwrap().to_string()
        } else {
            ModelController::get_temporary_local_read_url(library_id, media_id).await?
        };

        let videos_infos = probe_video(&uri).await?;

        let mut update = MediaForUpdate::default();
        if let Some(duration) = videos_infos.duration() {
            update.duration = Some(duration as u64);
        }
        let (width, height) = videos_infos.size();
        update.width = width;
        update.height = height;

        if let Some(video_stream) = videos_infos.video_stream() {
            update.color_space = video_stream.color_space.clone();
            update.vcodecs = video_stream.codec_name.clone().and_then(|c| Some(vec![c]));
            update.bitrate = video_stream.bitrate().clone();
        }

        self.update_media(library_id, media_id.to_owned(), update, requesting_user).await?;

        println!("videos infos {:?}", videos_infos);
        Ok(())
    }

    pub async fn update_photo_infos(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> crate::Result<()> {
        requesting_user.check_file_role(library_id, media_id, LibraryRole::Read)?;
        self.cache_check_library_notcrypt(library_id).await?;
        let mut m = self.library_file(library_id, media_id, None, MediaFileQuery::default() , requesting_user).await?.into_reader(library_id, None, None, Some((self.clone(), &requesting_user))).await?;

        let images_infos = image_tools::ImageCommandBuilder::new().infos(&mut m.stream).await?;
        if let Some(infos) = images_infos.get(0) {
            let mut update = MediaForUpdate::default();

            update.width = Some(infos.image.geometry.width);
            update.height = Some(infos.image.geometry.height);

            if let Some(color_space) = &infos.image.colorspace {
                update.color_space = Some(color_space.clone());
            }
        
    
            self.update_media(library_id, media_id.to_owned(), update, requesting_user).await?;
        }
        Ok(())
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
