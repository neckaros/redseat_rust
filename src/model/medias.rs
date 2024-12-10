


use std::{io::{self, Cursor, Read}, path::PathBuf, pin::Pin, result, str::FromStr};


use async_zip::{tokio::write::ZipFileWriter, Compression, ZipEntryBuilder};
use chrono::{Datelike, Utc};
use futures::{channel::mpsc::Sender, TryStreamExt};
use http::header::CONTENT_TYPE;
use mime::{Mime, APPLICATION_OCTET_STREAM};
use mime_guess::get_mime_extensions_str;
use nanoid::nanoid;
use query_external_ip::SourceError;
use rs_plugin_common_interfaces::{request::{RsRequest, RsRequestStatus}, url::{RsLink, RsLinkType}, PluginType};
use rusqlite::{types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, ToSql};
use serde::{Deserialize, Serialize};
use strum_macros::EnumString;
use tokio::{fs::File, io::{copy, AsyncRead, AsyncReadExt, AsyncWriteExt}, sync::mpsc};
use tokio_stream::StreamExt;
use tokio_util::{compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt}, io::{ReaderStream, StreamReader}};
use zip::ZipWriter;

use crate::{domain::{deleted::RsDeleted, library::LibraryType, media::{self, ConvertMessage, ConvertProgress, RsGpsPosition, DEFAULT_MIME}, plugin, MediaElement}, model::store::sql::SqlOrder, plugins::sources::{path_provider::PathProvider, Source}, routes::infos, tools::{file_tools::{filename_from_path, remove_extension}, image_tools::convert_image_reader, video_tools::{VideoCommandBuilder, VideoConvertRequest, VideoOverlayPosition}}};

use crate::{domain::{library::LibraryRole, media::{FileType, GroupMediaDownload, Media, MediaDownloadUrl, MediaForAdd, MediaForInsert, MediaForUpdate, MediaItemReference, MediaWithAction, MediasMessage, ProgressMessage}, progress::{RsProgress, RsProgressType}, ElementAction}, error::RsResult, plugins::{get_plugin_fodler, sources::{async_reader_progress::ProgressReader, error::SourcesError, AsyncReadPinBox, FileStreamResult, SourceRead}}, routes::mw_range::RangeDefinition, server::get_server_port, tools::{auth::{sign_local, ClaimsLocal}, file_tools::{file_type_from_mime, get_extension_from_mime}, image_tools::{self, resize_image, resize_image_reader, ImageSize, ImageType}, log::{log_error, log_info, LogServiceType}, prediction::{predict_net, preload_model, PredictionTagResult}, video_tools::{self, probe_video, VideoTime}}};

use super::{error::{Error, Result}, plugins::PluginQuery, store::{self, sql::library::medias::MediaBackup}, users::ConnectedUser, ModelController, VideoConvertQueueElement};

pub const CRYPTO_HEADER_SIZE: u64 = 16 + 4 + 4 + 32 + 256;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct MediaQuery {
    
    #[serde(default)]
    pub sort: RsSort,
    #[serde(default)]
    pub order: SqlOrder,

    pub added_before: Option<i64>,
    pub added_after: Option<i64>,

    pub created_before: Option<i64>,
    pub created_after: Option<i64>,

    pub modified_before: Option<i64>,
    pub modified_after: Option<i64>,

    pub before: Option<i64>,
    pub after: Option<i64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub tags_confidence: Option<u16>,
    #[serde(default)]
    pub people: Vec<String>,
    #[serde(default)]
    pub series: Vec<String>,
    pub movie: Option<String>,
    pub limit: Option<usize>,
    #[serde(default)]
    pub types: Vec<FileType>,

    
    pub text: Option<String>,

    pub long: Option<f64>,
    pub lat: Option<f64>,
    pub distance: Option<f64>,
    #[serde(default)]
    pub gps_square: Vec<f64>,
    
    pub min_duration: Option<u64>,
    pub max_duration: Option<u64>,

    pub min_rating: Option<f32>,
    pub max_rating: Option<f32>,

    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    
    
    pub page_key: Option<i64>,

    /// For legacy if user put serialized query in filter field
    pub filter: Option<String>,
}

impl FromSql for MediaQuery {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {

            let r = serde_json::from_str::<MediaQuery>(&as_string).map_err(|_| FromSqlError::InvalidType)?;

            Ok(r)
        })
    }
}

impl ToSql for MediaQuery {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let r = serde_json::to_string(&self).map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        Ok(ToSqlOutput::from(r))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct MediaFileQuery {
    #[serde(default)]
    pub unsupported_mime: Vec<String>,
    
    pub page: Option<u32>,

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
    pub fn from_after(after: i64) -> MediaQuery {
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
            mime: value.mimetype,
        })
    }
}

impl ModelController {

	pub async fn get_medias(&self, library_id: &str, query: MediaQuery, requesting_user: &ConnectedUser) -> Result<Vec<Media>> {
        let limits = requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let people = store.get_medias(query, limits).await?;
		Ok(people)
	}

	pub async fn count_medias(&self, library_id: &str, query: MediaQuery, requesting_user: &ConnectedUser) -> Result<u64> {
        let limits = requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let count = store.count_medias(query, limits).await?;
		Ok(count)
	}
    pub async fn get_locs(&self, library_id: &str, precision: Option<u32>, requesting_user: &ConnectedUser) -> Result<Vec<RsGpsPosition>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let locs = store.get_medias_locs(precision.unwrap_or(2)).await?;
		Ok(locs)
	}

    pub async fn get_medias_to_backup(&self, library_id: &str, after: i64, query: MediaQuery, requesting_user: &ConnectedUser) -> Result<Vec<MediaBackup>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let medias = store.get_all_medias_to_backup(after, query).await?;
		Ok(medias)
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

    pub async fn get_media_by_hash(&self, library_id: &str, hash: String, requesting_user: &ConnectedUser) -> RsResult<Option<Media>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let mut media = store.get_media_by_hash(hash).await;
        
        if let Some(media) = &mut media {
            if requesting_user.is_admin() {
                media.source = None;
            }
        }
		Ok(media)
	}

    pub async fn update_media(&self, library_id: &str, media_id: String, mut update: MediaForUpdate, notif: bool, requesting_user: &ConnectedUser) -> RsResult<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        if let Some(origin) = &update.origin_url {
            update.origin = Some(self.exec_parse(Some(library_id.to_owned()), origin.to_owned(), requesting_user).await?)
        }
		store.update_media(&media_id, update, requesting_user.user_id().ok()).await?;
        let media = store.get_media(&media_id).await?.ok_or(Error::NotFound)?;
        if notif {
            self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: media.clone(), action: ElementAction::Updated}] });
        }
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

    pub fn send_convert_progress(&self, message: ConvertMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("convert_progress", message);
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

    pub async fn remove_media(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> RsResult<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_media(media_id).await?;
        if let Some(existing) = existing { 
            self.remove_library_file(library_id, media_id, requesting_user).await?;
            self.add_deleted(library_id, RsDeleted::media(media_id.to_owned()), requesting_user).await?;
            self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: existing.clone(), action: ElementAction::Deleted}] });
            Ok(existing)
        } else {
            Err(Error::NotFound.into())
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


        let size = size.filter(|s| !(s == &ImageSize::Large || s == &ImageSize::Small));

        let result = self.library_image(library_id, ".thumbs", media_id, None, size.clone(), requesting_user).await;
        if let Err(error) = result {
            if let crate::Error::Source(SourcesError::NotFound(_)) = &error {
                self.generate_thumb(library_id, media_id, requesting_user).await.map_err(|_| Error::NotFound)?;
                self.library_image(library_id, ".thumbs", media_id, None, size, requesting_user).await

            } else {
                Err(error)
            }
        } else {
            result
        }
        
	}

    pub async fn split_media(&self, library_id: &str, media_id: String, from: u32, to: u32, requesting_user: &ConnectedUser) -> RsResult<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let media = store.get_media(&media_id).await?.ok_or(Error::NotFound)?;
        let mut infos: MediaForUpdate = media.clone().into();
        if media.kind != FileType::Album {
            return Err(Error::ServiceError("SPLIT".to_string(), Some(format!("{} media is not an albul", media_id))).into());
        }
        let filename = format!("{}-{}-{}.zip", media.name.replace(".zip", "").replace(".cbz", ""), from, to);
        println!("Zip name: {}", filename);
        
        let m = self.source_for_library(&library_id).await?;
        let (source, mut file) = m.writerseek(&filename).await?;
        //let file = file.compat_write();
        
        

        //let mut file = Box::pin(File::create("D:\\System\\backup\\2024\\7\\foo.zip").await?);
        let mut zip_writer = ZipFileWriter::with_tokio(&mut file);
        
        let mut pages = 0usize;
        let mut total_size = 0u64;
        for n in from..=to {
            
            
            let file = self.library_file(library_id, &media_id, 
                None, MediaFileQuery { page: Some(n) , ..Default::default()}, requesting_user).await?;
            let filename = file.filename().unwrap_or(nanoid!());
            let mut reader = file.into_reader(library_id, None, None, Some((self.clone(), requesting_user)), None).await?;

            let mut buffer = vec![];
            reader.stream.read_to_end(&mut buffer).await?;

            total_size += buffer.len() as u64;

            let builder = ZipEntryBuilder::new(filename.into(), Compression::Deflate);
            zip_writer.write_entry_whole(builder, &buffer).await.map_err(|_| Error::ServiceError("Unable to save zip entry".to_string(), None))?;
            pages += 1;


        }

        zip_writer.close().await.map_err(|_| Error::ServiceError("Unable to close zip file".to_string(), None))?;
        file.flush().await?;
        println!("CLOSED");

        m.fill_infos(&source, &mut infos).await?;

        if let Some(hash) = &infos.md5 {
            let existing = store.get_media_by_hash(hash.to_owned()).await;
            if let Some(existing) = existing {
                m.remove(&source).await?;
                //let _ = tx_progress.send(RsProgress { id: upload_id.clone(), total: existing.size, current: existing.size, kind: RsProgressType::Duplicate(existing.id.clone()), filename: Some(existing.name.to_owned()) }).await;
                return Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Media(existing)).into())
            }
        }

        let mut new_file = MediaForAdd::default();
        new_file.name = filename.to_string();
        new_file.source = Some(source.to_string());
        new_file.mimetype = infos.mimetype.clone().unwrap_or(DEFAULT_MIME.to_owned());
        new_file.created = Some(infos.created.unwrap_or_else(|| Utc::now().timestamp_millis()));
        new_file.pages = Some(pages);

        new_file.kind = FileType::Album;

        let id = nanoid!();
        store.add_media(MediaForInsert { id: id.clone(), media: new_file }).await?;
        self.update_media(library_id, id.to_owned(), infos, false, requesting_user).await?;
        let r = self.generate_thumb(&library_id, &id, &ConnectedUser::ServerAdmin).await;
        if let Err(r) = r {
            log_error(crate::tools::log::LogServiceType::Source, format!("Unable to generate thumb {:#}", r));
        }

        let media = store.get_media(&id).await?.ok_or(Error::NotFound)?;

        Ok(media)
	}

    pub async fn update_media_image<T: AsyncRead>(&self, library_id: &str, media_id: &str, reader: T, requesting_user: &ConnectedUser) -> Result<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        self.update_library_image(library_id, ".thumbs", media_id, &None, reader, requesting_user).await?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        store.update_media_thumb(media_id.to_owned()).await?;
        let media = self.get_media(library_id, media_id.to_owned(), requesting_user).await?.ok_or(Error::NotFound)?;
        self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media, action: ElementAction::Updated}] });
        Ok(())

	}

    
	pub async fn library_file(&self, library_id: &str, media_id: &str, mut range: Option<RangeDefinition>, query: MediaFileQuery, requesting_user: &ConnectedUser) -> RsResult<SourceRead> {
        requesting_user.check_file_role(library_id, media_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_media_source(&media_id).await?;
        let crypted = self.cache_get_library(library_id).await.and_then(|l| l.crypt).unwrap_or(false);
        if let Some(existing) = existing {
            let m = self.source_for_library(library_id).await?;
            if crypted && !query.raw {
                range = Some(RangeDefinition { start: Some(CRYPTO_HEADER_SIZE + existing.thumb_size.unwrap_or(0)), end: None })
            }
            let mut reader_response = m.get_file(&existing.source, range.clone()).await?;

            if existing.kind == FileType::Album && !query.raw {
                let local_path = m.local_path(&existing.source);
                if let Some(local_path) = local_path {
                    let archive = std::fs::File::open(local_path)?;
                    let buffreader = std::io::BufReader::new(archive);
                    let mut archive = zip::ZipArchive::new(buffreader).unwrap();

                    let mut file = archive.by_index(query.page.unwrap_or(1) as usize - 1).unwrap();

                    let mut data = Vec::new();
                    file.read_to_end(&mut data)?;
                    
                    let async_reader: AsyncReadPinBox = Box::pin(std::io::Cursor::new(data));
                    let source_reader = SourceRead::Stream(FileStreamResult { stream: async_reader, size: Some(file.size()), accept_range: false, range: None, mime: None, name: file.enclosed_name().map(|s| s.to_str().unwrap_or_default().to_string()), cleanup: None });

                    return Ok(source_reader);
                    
                    
                }
                


            }


            if crypted {
                if let SourceRead::Stream(reader) = &mut reader_response {
                    reader.range = None;
                    reader.size = existing.size;
                }
            }

            if !query.unsupported_mime.is_empty() && !query.raw {
                if existing.kind == FileType::Photo && query.unsupported_mime.contains(&existing.mime) || query.unsupported_mime.contains(&"all".to_owned()) {
                    let mut data = reader_response.into_reader(library_id, range, None, Some((self.clone(), &requesting_user)), None).await?; 
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
    pub fn process_media_spawn(&self, library_id: String, media_id: String, thumb: bool, predict: bool, requesting_user: ConnectedUser){
        let mc = self.clone();
        tokio::spawn(async move {
            let r = mc.process_media(&library_id, &media_id, thumb, predict, &requesting_user).await;
            if let Err(error) = r {
                log_error(crate::tools::log::LogServiceType::Source, format!("Unable to process media {} for predictions: {:?}", media_id, error));
            }
        });
    }
    pub async fn process_media(&self, library_id: &str, media_id: &str, thumb: bool, predict: bool, requesting_user: &ConnectedUser) -> RsResult<()>{
        if thumb {
            let _ = self.generate_thumb(library_id, media_id, requesting_user).await;
        }

        self.cache_check_library_notcrypt(library_id).await?;

        let existing = self.get_media(library_id, media_id.to_owned(), requesting_user).await?.ok_or(Error::NotFound)?;

        if existing.kind == FileType::Video {
            let r = self.update_video_infos(library_id, media_id, requesting_user, false).await;
            if let Err(r) = r {
                log_error(LogServiceType::Source, format!("unable to get video infos for {}: {:?}", media_id, r));
            }
        } else if existing.kind == FileType::Photo {
            let r = self.update_photo_infos(library_id, media_id, requesting_user, false).await;
            if let Err(r) = r {
                log_error(LogServiceType::Source, format!("unable to get photos infos for {}: {:?}", media_id, r));
            }
        }
    
        if predict {
            self.prediction(library_id, media_id, true, requesting_user, false).await?;
        }
        let existing = self.get_media(library_id, media_id.to_owned(), requesting_user).await?.ok_or(Error::NotFound)?;
        self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: existing, action: ElementAction::Updated}] });
        
        Ok(())
    }


    
    pub async fn add_library_file<T: Sized + AsyncRead + Send + Unpin >(&self, library_id: &str, filename: &str, infos: Option<MediaForUpdate>, reader: T, requesting_user: &ConnectedUser) -> RsResult<Media> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let mut infos = infos.unwrap_or_default();
        let upload_id = infos.upload_id.clone().unwrap_or_else(|| nanoid!());
        let crypted = self.cache_get_library_crypt(library_id).await;
        let m = self.source_for_library(&library_id).await?;

        let tx_progress = self.create_progress_sender(library_id.to_owned(), Some(upload_id.clone()));

        let mut progress_reader = ProgressReader::new(reader, RsProgress { id: upload_id.clone(), total: infos.size, current: Some(0), kind: RsProgressType::Transfert, filename: Some(filename.to_owned()) }, tx_progress.clone());

           
        let (source, mut file) = m.writer(filename, infos.size, infos.mimetype.clone()).await?;
        copy(&mut progress_reader, &mut file).await?;
        file.flush().await?;
        file.shutdown().await?;
        let source = source.await?;
        println!("source: {}", source);
        drop(progress_reader);
        if !crypted {
            let _ = m.fill_infos(&source, &mut infos).await;
        }
        if let Some(hash) = &infos.md5 {
            let existing = store.get_media_by_hash(hash.to_owned()).await;
            if let Some(existing) = existing {
                m.remove(&source).await?;
                let _ = tx_progress.send(RsProgress { id: upload_id.clone(), total: existing.size, current: existing.size, kind: RsProgressType::Duplicate(existing.id.clone()), filename: Some(existing.name.to_owned()) }).await;
                return Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Media(existing)).into())
            }
        }
        
        let mut new_file = MediaForAdd {
            name: filename.to_string(), 
            source: Some(source.to_string()),
            mimetype: infos.mimetype.clone().unwrap_or(DEFAULT_MIME.to_owned()),
            created: Some(infos.created.unwrap_or_else(|| Utc::now().timestamp_millis())),
            iv: infos.iv.clone(),
            thumbsize: infos.thumbsize,
            uploader: requesting_user.user_id().ok(),
            ..Default::default() };

        let message = ProgressMessage {
            library: library_id.to_owned(),
            progress: RsProgress { id: upload_id, total: infos.size, current: infos.size, kind: RsProgressType::Finished, filename: Some(new_file.name.clone()) },
            ..Default::default()
        };
        self.send_progress(message);
        
        new_file.kind = file_type_from_mime(&new_file.mimetype);

        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let id = nanoid!();
        store.add_media(MediaForInsert { id: id.clone(), media: new_file }).await?;
        self.update_media(library_id, id.to_owned(), infos, false, requesting_user).await?;
        let library: crate::model::libraries::ServerLibraryForRead = self.get_library(library_id, &ConnectedUser::ServerAdmin).await?.ok_or(Error::LibraryNotFound(library_id.to_owned()))?;
        if !crypted {
            let _ = self.generate_thumb(&library_id, &id, &requesting_user).await;
            self.process_media_spawn(library_id.to_string(), id.clone(), false, library.kind == LibraryType::Photos, requesting_user.clone());
        }


        let media = store.get_media(&id).await?.ok_or(Error::NotFound)?;
        self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: media.clone(), action: ElementAction::Added}] });

        Ok(media)
	}


    pub async fn download_library_url(&self, library_id: &str, files: GroupMediaDownload<MediaDownloadUrl>, requesting_user: &ConnectedUser) -> RsResult<Vec<Media>> {

        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        self.cache_check_library_notcrypt(library_id).await?;

        let m = self.source_for_library(library_id).await?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let mut medias: Vec<Media> = vec![];
        let origin = if let Some(origin) = &files.origin_url {
            Some(self.exec_parse(Some(library_id.to_owned()), origin.to_owned(), requesting_user).await.ok().unwrap_or(RsLink {platform: "link".to_owned(), kind: Some(RsLinkType::Post), id: origin.to_owned(), ..Default::default()}))
        } else {
            None
        };


        if files.group.unwrap_or_default() {
            let mut infos: MediaForUpdate = files.clone().into();
            infos.origin = origin.clone();

            let mut medias: Vec<Media> = vec![];
            let requests: Vec<RsRequest> = files.clone().into(); 

            let upload_id = nanoid!();
            //Progress
            let tx_progress = self.create_progress_sender(library_id.to_owned(), Some(upload_id.clone()));


            if let Some(origin) = &mut infos.origin {
                let origin_filename = if let Some(origin_url) = files.origin_url { filename_from_path(&origin_url) } else { None };
                origin.file = origin_filename;
                if !infos.ignore_origin_duplicate {
                    let existing = store.get_media_by_origin(origin.clone()).await;
                    if let Some(existing) = existing {
                        let _ = tx_progress.send(RsProgress { id: upload_id.clone(), total: existing.size, current: existing.size, kind: RsProgressType::Duplicate(existing.id.clone()), filename: Some(existing.name.to_owned()) }).await;
                        return Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Media(existing)).into())
                    }
                }
                if origin.user.is_none() {
                    if let Some(user) = &infos.people_lookup {
                        if let Some(user) = user.first() {
                            origin.user = Some(user.to_string())
                        }
                    }
                }
            }

            let filename = format!("{}.zip", nanoid!());
            println!("Zip name: {}", filename);
            let (source_porimise, mut file) = m.writer(&filename, None, None).await?;
            //let file = file.compat_write();
            
            

            //let mut file = Box::pin(File::create("D:\\System\\backup\\2024\\7\\foo.zip").await?);
            let mut zip_writer = ZipFileWriter::with_tokio(&mut file);

            let mut pages = 0usize;
            let mut total_size = 0u64;
            let len = requests.len() as u64;
            for request in requests {
                
                
                let reader = SourceRead::Request(request.clone()).into_reader(library_id, None, 
                    None, Some((self.clone(), &ConnectedUser::ServerAdmin)), None).await;

                if let Ok(mut reader) = reader {

                    let mut buffer = vec![];
                    reader.stream.read_to_end(&mut buffer).await?;

                    total_size += buffer.len() as u64;

                    let builder = ZipEntryBuilder::new(request.filename_or_extract_from_url().unwrap_or(nanoid!()).into(), Compression::Deflate);
                    zip_writer.write_entry_whole(builder, &buffer).await.map_err(|_| Error::ServiceError("Unable to save zip entry".to_string(), None))?;
                    pages += 1;
                    let estimated = total_size / pages as u64 * len;
                    let _ = tx_progress.send(RsProgress { id: upload_id.clone(), total: Some(estimated), current: Some(total_size), kind: RsProgressType::Download, filename: Some(filename.clone()) }).await;

                } else {
                    log_error(LogServiceType::Source, format!("Error adding to zip: {}", request.url));
                    println!("{:?}", reader);
                }
            }

            zip_writer.close().await.map_err(|_| Error::ServiceError("Unable to close zip file".to_string(), None))?;
            file.flush().await?;
            println!("CLOSED");

            let source = source_porimise.await?;

            m.fill_infos(&source, &mut infos).await?;

            if let Some(hash) = &infos.md5 {
                let existing = store.get_media_by_hash(hash.to_owned()).await;
                if let Some(existing) = existing {
                    m.remove(&source).await?;
                    //let _ = tx_progress.send(RsProgress { id: upload_id.clone(), total: existing.size, current: existing.size, kind: RsProgressType::Duplicate(existing.id.clone()), filename: Some(existing.name.to_owned()) }).await;
                    return Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Media(existing)).into())
                }
            }

            let mut new_file = MediaForAdd::default();
            new_file.name = filename.to_string();
            new_file.source = Some(source.to_string());
            new_file.mimetype = infos.mimetype.clone().unwrap_or(DEFAULT_MIME.to_owned());
            new_file.created = Some(infos.created.unwrap_or_else(|| Utc::now().timestamp_millis()));
            new_file.pages = Some(pages);

            new_file.kind = FileType::Album;

            let id = nanoid!();
            store.add_media(MediaForInsert { id: id.clone(), media: new_file }).await?;
            self.update_media(library_id, id.to_owned(), infos, false, requesting_user).await?;
            let r = self.generate_thumb(&library_id, &id, &ConnectedUser::ServerAdmin).await;
            if let Err(r) = r {
                log_error(crate::tools::log::LogServiceType::Source, format!("Unable to generate thumb {:#}", r));
            }

            let media = store.get_media(&id).await?.ok_or(Error::NotFound)?;
            let _ = tx_progress.send(RsProgress { id: upload_id.clone(), total: media.size, current: media.size, kind: RsProgressType::Finished, filename: Some(media.name.to_owned()) }).await;

            self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: media.clone(), action: ElementAction::Added}] });
            
            medias.push(media);


            Ok(medias)
        } else {
            let requests: Vec<RsRequest> = files.into(); 
        
            //let infos = infos.unwrap_or_else(|| MediaForUpdate::default());
            for mut request in requests {
                let upload_id = request.upload_id.clone().unwrap_or_else(|| nanoid!());

                self.plugin_manager.fill_infos(&mut request).await;
                let mut infos: MediaForUpdate = request.clone().into();
                infos.origin = origin.clone();
                
                
                
                //Progress
                let tx_progress = self.create_progress_sender(library_id.to_owned(), Some(upload_id.clone()));

                
                if let Some(origin) = &mut infos.origin {
                    let origin_filename = filename_from_path(&request.url);
                    origin.file = origin_filename;
                    if !infos.ignore_origin_duplicate {
                        let existing = store.get_media_by_origin(origin.clone()).await;
                        if let Some(existing) = existing {
                            let _ = tx_progress.send(RsProgress { id: upload_id.clone(), total: existing.size, current: existing.size, kind: RsProgressType::Duplicate(existing.id.clone()), filename: Some(existing.name.to_owned()) }).await;
                            println!("duplicate origin!");
                            return Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Media(existing)).into())
                        }
                    }
                }


                let reader = SourceRead::Request(request).into_reader(library_id, None, 
                    Some(tx_progress.clone()), Some((self.clone(), &ConnectedUser::ServerAdmin)), None).await?;

                if let Some(reader_name) = reader.name {
                    infos.name = Some(reader_name);
                }
                let mut filename =infos.name.clone().unwrap_or(nanoid!());
                if infos.mimetype.is_none() {
                    infos.mimetype = reader.mime;
                }

                if !filename.contains(".") || filename.split(".").last().unwrap_or("").len() > 5 {
                    if let Some(mimetype) = &infos.mimetype {
                        let suffix = get_extension_from_mime(mimetype);
                        filename = format!("{}.{}", filename, suffix);
                    }
                }
                
                
                let mut progress_reader = ProgressReader::new(reader.stream, RsProgress { id: upload_id.clone(), total: reader.size, current: Some(0), kind: RsProgressType::Transfert, filename: Some(filename.clone()) }, tx_progress.clone());
                let (source, mut file) = m.writerseek(&filename).await?;
                copy(&mut progress_reader, &mut file).await?;
                file.flush().await?;
                drop(progress_reader);
            
                let _ = m.fill_infos(&source, &mut infos).await;

                if let Some(hash) = &infos.md5 {
                    let existing = store.get_media_by_hash(hash.to_owned()).await;
                    if let Some(existing) = existing {
                        m.remove(&source).await?;
                        let _ = tx_progress.send(RsProgress { id: upload_id.clone(), total: existing.size, current: existing.size, kind: RsProgressType::Duplicate(existing.id.clone()), filename: Some(existing.name.to_owned()) }).await;
                        return Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Media(existing)).into())
                    }
                }

                let mut new_file = MediaForAdd::default();
                new_file.name = filename.to_string();
                new_file.source = Some(source.to_string());
                new_file.mimetype = infos.mimetype.clone().unwrap_or(DEFAULT_MIME.to_owned());
                new_file.created = Some(infos.created.unwrap_or_else(|| Utc::now().timestamp_millis()));

                let final_progress = tx_progress.send(RsProgress { id: upload_id.clone(), total: infos.size, current: infos.size, kind: RsProgressType::Analysing, filename: Some(filename.to_owned()) }).await;
                if let Err(error) = final_progress {
                    log_error(LogServiceType::Source, format!("Unable to send final progress message: {:?}", error));
                }
                
                new_file.kind = file_type_from_mime(&new_file.mimetype);

                let id = nanoid!();
                store.add_media(MediaForInsert { id: id.clone(), media: new_file }).await?;
                self.update_media(library_id, id.to_owned(), infos, false, requesting_user).await?;

                let r = self.generate_thumb(&library_id, &id, &ConnectedUser::ServerAdmin).await;
                let library = self.get_library(library_id, &ConnectedUser::ServerAdmin).await?.ok_or(Error::LibraryNotFound(library_id.to_owned()))?;
                self.process_media_spawn(library_id.to_string(), id.clone(), false, library.kind == LibraryType::Photos, ConnectedUser::ServerAdmin);
                
                if let Err(r) = r {
                    log_error(crate::tools::log::LogServiceType::Source, format!("Unable to generate thumb {:#}", r));
                }
                let media = store.get_media(&id).await?.ok_or(Error::NotFound)?;
                let _ = tx_progress.send(RsProgress { id: upload_id.clone(), total: media.size, current: media.size, kind: RsProgressType::Finished, filename: Some(media.name.to_owned()) }).await;
                self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: media.clone(), action: ElementAction::Added}] });
                
                medias.push(media)


            }
            Ok(medias)
        }
	}


    pub fn create_progress_sender(&self, library_id: String, upload_id: Option<String>) -> mpsc::Sender<RsProgress> {
        //Progress
        let mc_progress = self.clone();
        let (tx_progress, mut rx_progress) = mpsc::channel::<RsProgress>(100);
        tokio::spawn(async move {
            let mut last_send = 0;
            let mut last_type: Option<RsProgressType> = None;
            
            let start_time = std::time::Instant::now();

            let mut remaining = None;

            while let Some(mut progress) = rx_progress.recv().await {
                let percent = progress.percent();
                if let Some(percent) = percent {
                    if percent > 0.01f32 {
                        let duration_from_start = start_time.elapsed().as_secs_f32();
                        let remaining_time = duration_from_start / percent * (1f32 - percent);
                        //println!("FFMPEG remaining time: {}", remaining_time);
                        
                        remaining = Some(remaining_time as u64);
                    }
                }

                let current = progress.current.unwrap_or(1);
                if let Some(upload_id) = &upload_id {
                    progress.id = upload_id.clone();
                }
                if progress.current == progress.total || last_send == 0 || current < last_send || current - last_send  > 1000000 || Some(&progress.kind) != last_type.as_ref() {
                    last_type = Some(progress.kind.clone());
                    last_send = current;
                    let message = ProgressMessage {
                        library: library_id.clone(),
                        progress,
                        remaining_secondes: remaining
                    };
                    mc_progress.send_progress(message);
                }
            }
        });
        tx_progress
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
        new_file.name = final_infos.name.or(infos.name).unwrap_or(nanoid!());
        new_file.source = if let Some(selected) = request.selected_file { Some(format!("{}|{}",request.url, selected)) } else { Some(request.url) };
        new_file.mimetype = final_infos.mimetype.or(infos.mimetype).unwrap_or(DEFAULT_MIME.to_owned());
        new_file.size = final_infos.size.or(infos.size);
        new_file.created = Some(infos.created.unwrap_or_else(|| Utc::now().timestamp_millis()));
        
        new_file.kind = file_type_from_mime(&new_file.mimetype);
        

        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let id = nanoid!();
        store.add_media(MediaForInsert { id: id.clone(), media: new_file }).await?;
        
        if let Some(update) = additional_infos {
            self.update_media(library_id, id.clone(), update, false, requesting_user).await?;
        }
        let library = self.get_library(library_id, &ConnectedUser::ServerAdmin).await?.ok_or(Error::LibraryNotFound(library_id.to_owned()))?;
        if !library.crypt.unwrap_or_default() {
            let _ = self.generate_thumb(&library_id, &id, &requesting_user).await;
            self.process_media_spawn(library_id.to_string(), id.clone(), true, library.kind == LibraryType::Photos, requesting_user.clone());
        }

        let media = store.get_media(&id).await?.ok_or(Error::MediaNotFound(id.to_owned()))?;
        self.send_media(MediasMessage { library: library_id.to_string(), medias: vec![MediaWithAction { media: media.clone(), action: ElementAction::Added}] });

        Ok(media)
	}


    pub async fn generate_thumb(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> crate::error::Result<()> {
        self.cache_check_library_notcrypt(library_id).await?;

        let m = self.source_for_library(library_id).await?; 
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let media = store.get_media(media_id).await?.ok_or(Error::NotFound)?;
        
        let thumb = match media.kind {
            FileType::Photo => { 
                let media_source: MediaSource = media.try_into()?;
                println!("Photo {}", &media_source.source);
                let reader = m.get_file(&media_source.source, None).await?;
                let mut reader = reader.into_reader(library_id, None, None, Some((self.clone(), &requesting_user)), None).await?;
                let image = resize_image_reader(&mut reader.stream, 512).await?;
                Ok(image)
            },
            FileType::Album => { 
                let media_source: MediaSource = media.try_into()?;
                println!("Photo {}", &media_source.source);
                let reader = self.library_file(library_id, media_id, None, MediaFileQuery {page: Some(1), unsupported_mime: vec![], raw: false }, requesting_user).await?;
                let mut reader = reader.into_reader(library_id, None, None, Some((self.clone(), &requesting_user)), None).await?;
                let image = resize_image_reader(&mut reader.stream, 512).await?;
                Ok(image)
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

        println!("Got thumb {}", thumb.len());
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
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let exp = ClaimsLocal::generate_seconds(delay_in_seconds);
        let claims = ClaimsLocal {
            cr: "service::share_media".to_string(),
            kind: crate::tools::auth::ClaimsLocalType::File(library_id.to_string(), media_id.to_string()),
            exp,
        };
        let token = sign_local(claims).await.map_err(|_| Error::UnableToSignShareToken)?;
        Ok(token)
    }

    pub async fn prediction(&self, library_id: &str, media_id: &str, insert_tags: bool, requesting_user: &ConnectedUser, notif: bool) -> crate::Result<Vec<PredictionTagResult>> {
        let plugins = self.get_plugins(PluginQuery { kind: Some(PluginType::ImageClassification), library: Some(library_id.to_string()), ..Default::default() }, &ConnectedUser::ServerAdmin).await?;
        if !plugins.is_empty() {
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
                            self.update_media(&library_id, media_id.to_string(), MediaForUpdate { add_tags: Some(vec![MediaItemReference { id: db_tag.id, conf: Some(tag.probability as u16) }]), ..Default::default() }, notif, &requesting_user).await?;
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

    pub async fn convert(&self, library_id: &str, media_id: &str, request: VideoConvertRequest, requesting_user: &ConnectedUser) -> crate::Result<()> {
        requesting_user.check_file_role(library_id, media_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let media = store.get_media(media_id).await?.ok_or(Error::NotFound)?;


        let filename = format!("{}.{}", remove_extension(&media.name), request.format);
        let queue_element = VideoConvertQueueElement::new(library_id.to_string(), media_id.to_string(), filename, requesting_user.clone(), request);
        let message = ConvertMessage {
            library: library_id.to_string(),
            progress: queue_element.status.clone(),
        };
        self.send_convert_progress(message);

        let converting = self.convert_current.read().await;
        let mut queue = self.convert_queue.write().await;
        queue.push_back(queue_element);
        drop(queue);
        if !converting.to_owned() {
            drop(converting);
            let mc = self.clone();
            tokio::spawn(async move {
                let mut converting = mc.convert_current.write().await;
                *converting = true;
                loop  {
                    let mut queue = mc.convert_queue.write().await;
                    let element = queue.pop_front();
                    drop(queue);

                    if let Some(element) = element {
                        let _ = mc.convert_element(element).await;
                    } else {
                        break;
                    }
                }
                *converting = false;
                println!("Finished transcoding");
            });
        }

        Ok(())

    }

    pub async fn convert_element(&self, mut element: VideoConvertQueueElement) -> crate::Result<Media> {
        element.user.check_file_role(&element.library, &element.media, LibraryRole::Write)?;

        let store = self.store.get_library_store(&element.library).ok_or(Error::NotFound)?;
        let media = store.get_media(&element.media).await?.ok_or(Error::NotFound)?;

        let m = self.source_for_library(&element.library).await?; 
        let local = self.library_source_for_library(&element.library).await?; 
        let path = m.local_path(media.source.as_ref().ok_or(Error::ServiceError("Convert".to_owned(), Some("Unable to convert video without source".to_owned())))?).ok_or(Error::ServiceError("Convert".to_owned(), Some("Unable to convert video that is not local".to_owned())))?;
 
        let filename = element.status.filename;
        let dest_source = format!(".cache/{}", filename);
        let dest = local.get_full_path(&dest_source);
        PathProvider::ensure_filepath(&dest).await?;


        let mc_progress = self.clone();
        let lib_progress = element.library.to_string();
        let request_progress = element.request.clone();
        let name_progress = filename.clone();
        let (tx_progress, mut rx_progress) = mpsc::channel::<f64>(100);
        tokio::spawn(async move {

            let start_time = std::time::Instant::now();

            let mut remaining = None;
            while let Some(percent) = rx_progress.recv().await {     
                
                if percent > 0.01f64 {
                    let duration_from_start = start_time.elapsed().as_secs_f64();
                    let remaining_time = duration_from_start / percent * (1f64 - percent);
                    //println!("FFMPEG remaining time: {}", remaining_time);
                    
                    remaining = Some(remaining_time as u64);
                }
                


                let message = ConvertMessage {
                    library: lib_progress.clone(),
                    progress: ConvertProgress {
                        percent,
                        converted_id: None,
                        filename: name_progress.clone(),
                        done: false,
                        id: request_progress.id.clone(),
                        request: Some(request_progress.clone()),
                        estimated_remaining_seconds: remaining
                    },
                };
                mc_progress.send_convert_progress(message);
                
            }
        });

        let mut video_builder = VideoCommandBuilder::new();
        video_builder.set_progress(tx_progress);


        if let Some(overlay) = &mut element.request.overlay {
            match overlay.kind {
                video_tools::VideoOverlayType::Watermark => {
                    let name = if overlay.path.is_empty() { ".watermark.png".to_owned() } else { format!(".watermark.{}.png", &overlay.path)};
                    overlay.path = local.get_full_path(&name).to_str().ok_or(Error::ServiceError("Convert".to_owned(), Some("Invalid watermark path".to_owned())))?.to_string();
                },
                video_tools::VideoOverlayType::File => todo!(),
            }
  
        }
        let progress_id = element.request.id.clone();
        video_builder.set_request(element.request.clone()).await?;
        video_builder.run_file(path.to_str().unwrap(), dest.to_str().unwrap()).await?;
        let message = ConvertMessage {
            library: element.library.to_string(),
            progress: ConvertProgress {
                percent: 1.0,
                converted_id: None,
                filename: filename.clone(),
                done: true,
                id: progress_id.clone(),
                request: Some(element.request.clone()),
                ..Default::default()
            },
        };
        self.send_convert_progress(message);
        let media_infos: MediaForUpdate = media.into();
        let reader = File::open(dest).await?;
        let media = self.add_library_file(&element.library, &filename, Some(media_infos), reader, &element.user).await;
        
        local.remove(&dest_source).await?;
        match media {
            Ok(media) => {
                let message = ConvertMessage {
                    library: element.library.to_string(),
                    progress: ConvertProgress {
                        percent: 1.0,
                        converted_id: Some(media.id.clone()),
                        filename: filename.clone(),
                        done: true,
                        id: progress_id.clone(),
                        request: Some(element.request.clone()),
                        ..Default::default()
                    },
                };
                self.send_convert_progress(message);
                Ok(media)
            },
            Err(err) => Err(err),
        }
    }
    
    pub async fn update_video_infos(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser, notif: bool) -> crate::Result<()> {
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
            update.vcodecs = video_stream.codec_name.clone().map(|c| vec![c]);
            update.bitrate = video_stream.bitrate();
            update.fps = video_stream.fps()
        }

        self.update_media(library_id, media_id.to_owned(), update, notif, requesting_user).await?;

        //println!("videos infos {:?}", videos_infos);
        Ok(())
    }

    pub async fn update_photo_infos(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser, notif: bool) -> crate::Result<()> {
        requesting_user.check_file_role(library_id, media_id, LibraryRole::Read)?;
        self.cache_check_library_notcrypt(library_id).await?;
        let mut m = self.library_file(library_id, media_id, None, MediaFileQuery::default() , requesting_user).await?.into_reader(library_id, None, None, Some((self.clone(), &requesting_user)), None).await?;

        let images_infos = image_tools::ImageCommandBuilder::new().infos(&mut m.stream).await?;
        if let Some(infos) = images_infos.first() {
            let mut update = MediaForUpdate::default();

            update.mp = Some(u32::from(infos.image.geometry.width * infos.image.geometry.height / 1000000));


            update.width = Some(infos.image.geometry.width);
            update.height = Some(infos.image.geometry.height);
            update.orientation = infos.image.orientation();
            update.iso = infos.image.iso();
            update.focal = infos.image.focal();
            update.f_number = infos.image.f_number();
            update.model = infos.image.properties.exif_model.clone();
            update.sspeed = infos.image.properties.exif_exposure_time.clone();
            update.icc = infos.image.properties.icc_description.clone();
            

            if let Some(color_space) = &infos.image.colorspace {
                update.color_space = Some(color_space.clone());
            }
        
    
            self.update_media(library_id, media_id.to_owned(), update, notif, requesting_user).await?;
        }
        Ok(())
    }

    pub async fn remove_library_file(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
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
