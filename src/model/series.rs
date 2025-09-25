


use std::{io::{self, Cursor, Read}, pin::Pin};

use async_recursion::async_recursion;
use futures::TryStreamExt;
use nanoid::nanoid;
use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, lookup::RsLookupMovie, ExternalImage, ImageType};
use rusqlite::{types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, ToSql};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{fs::File, io::{AsyncRead, AsyncWriteExt, BufReader}};
use tokio_util::io::StreamReader;


use crate::{domain::{deleted::RsDeleted, library::LibraryRole, people::{PeopleMessage, Person}, serie::{Serie, SerieStatus, SerieWithAction, SeriesMessage}, ElementAction, MediaElement}, error::RsResult, plugins::{medias::imdb::ImdbContext, sources::{error::SourcesError, path_provider::PathProvider, AsyncReadPinBox, FileStreamResult, Source}}, server::get_server_folder_path_array, tools::{image_tools::{convert_image_reader, resize_image_reader, ImageSize}, log::log_info}};

use super::{episodes::{EpisodeForUpdate, EpisodeQuery}, error::{Error, Result}, medias::{MediaQuery, RsSort}, store::sql::SqlOrder, users::ConnectedUser, ModelController};


impl FromSql for SerieStatus {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            SerieStatus::try_from(&*as_string).map_err(|_| FromSqlError::InvalidType)
        })
    }
}

impl ToSql for SerieStatus {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SerieQuery {
    pub after: Option<i64>,

    pub name: Option<String>,
    
    #[serde(default)]
    pub sort: RsSort,
    #[serde(default)]
    pub order: SqlOrder,
}

impl SerieQuery {
    pub fn new_empty() -> SerieQuery {
        SerieQuery { after: None, ..Default::default() }
    }
    pub fn from_after(after: i64) -> SerieQuery {
        SerieQuery { after: Some(after), ..Default::default() }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SerieForUpdate {
	pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub status: Option<SerieStatus>,
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
    pub max_created: Option<i64>,
}

impl SerieForUpdate {
    pub fn has_update(&self) -> bool {
        self != &SerieForUpdate::default()
    } 
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ExternalSerieImages {
    pub backdrop: Option<String>,
    pub logo: Option<String>,
    pub poster: Option<String>,
    pub still: Option<String>,
    pub card: Option<String>,
    pub portraits: Option<String>,
}




impl ExternalSerieImages {
    pub fn into_kind(self, kind: ImageType) -> Option<String> {
        match kind {
            ImageType::Poster => self.poster,
            ImageType::Background => self.backdrop,
            ImageType::Still => self.still,
            ImageType::Card => self.card,
            ImageType::ClearLogo => self.logo,
            ImageType::ClearArt => None,
            ImageType::Custom(_) => None,
        }
    }
}

impl Serie {
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

	pub async fn get_series(&self, library_id: &str, query: SerieQuery, requesting_user: &ConnectedUser) -> RsResult<Vec<Serie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
		let people = store.get_series(query).await?;
		Ok(people)
	}

    pub async fn get_serie(&self, library_id: &str, serie_id: String, requesting_user: &ConnectedUser) -> RsResult<Option<Serie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;

        if RsIds::is_id(&serie_id) {
            let id: RsIds = serie_id.clone().try_into().map_err(|_| SourcesError::UnableToFindSerie(library_id.to_string(), format!("{:?}", serie_id), "get_serie".to_string()))?;
            let serie = store.get_serie_by_external_id(id.clone()).await?;
            if let Some(serie) = serie {
                Ok(Some(serie))
            } else {
                let mut trakt_show = self.trakt.get_serie(&id).await.map_err(|_| SourcesError::UnableToFindSerie(library_id.to_string(), format!("{:?}", id), "get_serie".to_string()))?;
                trakt_show.fill_imdb_ratings(&self.imdb).await;
                Ok(Some(trakt_show))
            }
        } else {
            let serie = store.get_serie(&serie_id).await?;
            Ok(serie)
        }
	}

    pub async fn get_serie_by_external_id(&self, library_id: &str, ids: RsIds, requesting_user: &ConnectedUser) -> RsResult<Option<Serie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let serie = store.get_serie_by_external_id(ids).await?;
        Ok(serie)
    }


    pub async fn get_serie_ids(&self, library_id: &str, serie_id: &str, requesting_user: &ConnectedUser) -> RsResult<RsIds> {
        let serie = self.get_serie(library_id, serie_id.to_string(), requesting_user).await?.ok_or(Error::LibraryStoreNotFoundFor(library_id.to_string(), "get_serie_ids".to_string()))?;
        let ids: RsIds = serie.into();
        Ok(ids)
    }

    pub async fn trending_shows(&self)  -> RsResult<Vec<Serie>> {
        self.trakt.trending_shows().await
    }

    pub async fn search_serie(&self, library_id: &str, query: RsLookupMovie, requesting_user: &ConnectedUser) -> RsResult<Vec<Serie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let searched = self.trakt.search_show(&query).await?;
		Ok(searched)
	}



    pub async fn update_serie(&self, library_id: &str, serie_id: String, update: SerieForUpdate, requesting_user: &ConnectedUser) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        if RsIds::is_id(&serie_id) {
            return Err(Error::InvalidIdForAction("udpate".to_string(), serie_id).into())
        }
        if update.has_update() {
            let store = self.store.get_library_store(library_id)?;
            store.update_serie(&serie_id, update).await?;
            let serie = store.get_serie(&serie_id).await?.ok_or(SourcesError::UnableToFindSerie(library_id.to_string(), serie_id, "get_serie".to_string()))?;
            self.send_serie(SeriesMessage { library: library_id.to_string(), series: vec![SerieWithAction { action: ElementAction::Updated, serie: serie.clone() }] });
            Ok(serie)
        } else {
            let serie = self.get_serie(library_id, serie_id.clone(), requesting_user).await?.ok_or(SourcesError::UnableToFindSerie(library_id.to_string(), serie_id, "get_serie".to_string()))?;
            Ok(serie)
        }  
	}


	pub fn send_serie(&self, message: SeriesMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("series", message);
			}
		});
	}


    pub async fn add_serie(&self, library_id: &str, mut new_serie: Serie, requesting_user: &ConnectedUser) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let ids: RsIds = new_serie.clone().into();
        let existing = self.get_serie_by_external_id(library_id, ids, requesting_user).await?;
        if let Some(existing) = existing {
            return Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Serie(existing)).into())
        }
        let store = self.store.get_library_store(library_id)?;
        let id = nanoid!();
        new_serie.id = id.clone();
		store.add_serie(new_serie).await?;
        let inserted_serie = self.get_serie(library_id, id.clone(), requesting_user).await?.ok_or(SourcesError::UnableToFindSerie(library_id.to_string(), id, "add_serie".to_string()))?;
        self.send_serie(SeriesMessage { library: library_id.to_string(), series: vec![SerieWithAction { action: ElementAction::Added, serie: inserted_serie.clone() }] });
        
        let mc = self.clone();
        let inserted_serie_id = inserted_serie.id.clone();
        let library_id = library_id.to_string();
        let requesting_user = requesting_user.clone();
        tokio::spawn(async move {
            mc.refresh_serie(&library_id, &inserted_serie_id, &requesting_user).await.unwrap();
            mc.refresh_episodes(&library_id, &inserted_serie_id, &requesting_user).await.unwrap();
        });
		Ok(inserted_serie)
	}


    pub async fn remove_serie(&self, library_id: &str, serie_id: &str, delete_medias: bool, requesting_user: &ConnectedUser) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if RsIds::is_id(serie_id) {
            return Err(Error::InvalidIdForAction("remove".to_string(), serie_id.to_string()).into())
        }
        let store = self.store.get_library_store(library_id)?;
        let existing = store.get_serie(serie_id).await?.ok_or(SourcesError::UnableToFindSerie(library_id.to_string(), serie_id.to_string(), "remove_serie".to_string()))?;
       
        if delete_medias {
            let medias = self.get_medias(library_id, MediaQuery { series: vec![existing.id.clone()], ..Default::default() }, requesting_user).await?;
            for media in medias {
                self.remove_media(library_id, &media.id, requesting_user).await?;
            }
        }


        store.remove_serie(serie_id.to_string()).await?;
        self.add_deleted(library_id, RsDeleted::serie(serie_id.to_owned()), requesting_user).await?;
        self.send_serie(SeriesMessage { library: library_id.to_string(), series: vec![SerieWithAction { action: ElementAction::Deleted, serie: existing.clone() }] });
        Ok(existing)

	}

    pub async fn refresh_serie(&self, library_id: &str, serie_id: &str, requesting_user: &ConnectedUser) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let ids = self.get_serie_ids(library_id, serie_id, requesting_user).await?;
        let serie = self.get_serie(library_id, serie_id.to_string(), requesting_user).await?.ok_or(SourcesError::UnableToFindSerie(library_id.to_string(), serie_id.to_string(), "remove_serie".to_string()))?;
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


    pub async fn refresh_series_imdb(&self, library_id: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
        let all_series = self.get_series(&library_id, SerieQuery::default(), &requesting_user).await?;
        //Imdb rating
        for mut serie in all_series {
            let existing_votes = serie.imdb_votes.unwrap_or(0);
            serie.fill_imdb_ratings(&self.imdb).await;
            let serieid = serie.id.clone();
            if existing_votes != serie.imdb_votes.unwrap_or(0) {
                self.update_serie(library_id, serie.id, SerieForUpdate { imdb_rating: serie.imdb_rating, imdb_votes: serie.imdb_votes, ..Default::default()}, &ConnectedUser::ServerAdmin).await?;
            }
            let episodes = self.get_episodes(library_id, EpisodeQuery {serie_ref: Some(serieid.clone()), ..Default::default() }, &ConnectedUser::ServerAdmin).await?;
            for mut episode in episodes {
                let existing_votes = episode.imdb_votes.unwrap_or(0);
                episode.fill_imdb_ratings(&self.imdb).await;
                if existing_votes != episode.imdb_votes.unwrap_or(0) {
                    self.update_episode(library_id, serieid.clone(), episode.season, episode.number, EpisodeForUpdate { imdb_rating: serie.imdb_rating, imdb_votes: serie.imdb_votes, ..Default::default()}, &ConnectedUser::ServerAdmin).await?;
                }
            }
        }
        Ok(())
    }


    pub async fn import_serie(&self, library_id: &str, serie_id: &str, requesting_user: &ConnectedUser) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if let Ok(ids) = RsIds::try_from(serie_id.to_string()) {
            let existing = self.get_serie_by_external_id(library_id, ids.clone(), requesting_user).await?;
            if let Some(existing) = existing {
                Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Serie(existing)).into())
            } else { 
                let mut new_serie = self.trakt.get_serie(&ids).await?;
                new_serie.fill_imdb_ratings(&self.imdb).await;
                let imported_serie = self.add_serie(library_id, new_serie, requesting_user).await?;
                Ok(imported_serie)
            }
        } else {
            
            Err(Error::InvalidIdForAction("import".to_string(), serie_id.to_string()).into())
        }
    
	}
    
    #[async_recursion]
	pub async fn serie_image(&self, library_id: &str, serie_id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> crate::Result<FileStreamResult<AsyncReadPinBox>> {
        let kind = kind.unwrap_or(ImageType::Poster);
        if RsIds::is_id(serie_id) {
            let mut serie_ids: RsIds = serie_id.to_string().try_into()?;

            let store = self.store.get_library_store(library_id)?;
            let existing_serie = store.get_serie_by_external_id(serie_ids.clone()).await?;
            if let Some(existing_serie) = existing_serie {
                let image = self.serie_image(library_id,  &existing_serie.id, Some(kind), size, requesting_user).await?;
                Ok(image)
            } else {

                let local_provider = self.library_source_for_library(library_id).await?;
                
                if serie_ids.tmdb.is_none() {
                    let serie = self.trakt.get_serie(&serie_ids).await?;
                    serie_ids = serie.into();
                }
                let image_path = format!("cache/serie-{}-{}.avif", serie_id.replace(':', "-"), kind);

                if !local_provider.exists(&image_path).await {
                    let images = self.get_serie_image_url(&serie_ids, &kind, &None).await?.ok_or(crate::Error::NotFound(format!("Unable to get series image url: {:?} kind {:?}",serie_ids, kind)))?;
                    let (_, mut writer) = local_provider.get_file_write_stream(&image_path).await?;
                    let image_reader = reqwest::get(images).await?;
                    let stream = image_reader.bytes_stream();
                    let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
                    let mut body_reader = StreamReader::new(body_with_io_error);
                    let resized = resize_image_reader(Box::pin(body_reader), ImageSize::Large.to_size(), image::ImageFormat::Avif, Some(70), false).await?;

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
        let serie = self.get_serie(library_id, serie_id.to_string(), requesting_user).await?.ok_or(SourcesError::UnableToFindSerie(library_id.to_string(), serie_id.to_string(), "refresh_serie_image".to_string()))?;
        let ids: RsIds = serie.into();
        let reader = self.download_serie_image(&ids, kind, &None).await?;
        self.update_serie_image(library_id, serie_id, kind, reader, &ConnectedUser::ServerAdmin).await?;
        Ok(())
	}

    pub async fn get_serie_image_url(&self, ids: &RsIds, kind: &ImageType, lang: &Option<String>) -> RsResult<Option<String>> {
        let images = if kind == &ImageType::Card {
            None
        } else { 
            self.tmdb.serie_image(ids.clone(), lang).await?.into_kind(kind.clone())
        };
        if images.is_none() {
            let images = self.fanart.serie_image(ids.clone()).await?.into_kind(kind.clone());
            Ok(images)
        } else {
            Ok(images)
        }
    }

    pub async fn get_serie_images(&self, ids: &RsIds) -> RsResult<Vec<ExternalImage>> {
        let mut images = self.tmdb.serie_images(ids.clone()).await?;
       
        let mut fanart = self.fanart.serie_images(ids.clone()).await?;
        images.append(&mut fanart);
        Ok(images)
    }


    pub async fn download_serie_image(&self, ids: &RsIds, kind: &ImageType, lang: &Option<String>) -> crate::Result<AsyncReadPinBox> {
        let images = self.get_serie_image_url(ids, kind, lang).await?.ok_or(crate::Error::NotFound(format!("Unable to get series image url: {:?} kind {:?}",ids, kind)))?;
        let image_reader = reqwest::get(images).await?;
        let stream = image_reader.bytes_stream();
        let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
        let body_reader = StreamReader::new(body_with_io_error);
        Ok(Box::pin(body_reader))
    }

    pub async fn update_serie_image(&self, library_id: &str, serie_id: &str, kind: &ImageType, mut reader: AsyncReadPinBox, requesting_user: &ConnectedUser) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if RsIds::is_id(serie_id) {
            return Err(Error::InvalidIdForAction("udpate image".to_string(), serie_id.to_string()).into())
        }

        let converted = convert_image_reader(reader, image::ImageFormat::Avif, Some(60), false).await?;
        let converted_reader = Cursor::new(converted);
        
        self.update_library_image(library_id, ".series", serie_id, &Some(kind.clone()), &None, converted_reader, requesting_user).await?;
        
        let store = self.store.get_library_store(library_id)?;
		store.update_serie_image(serie_id.to_string(), kind.clone()).await;

        let serie = self.get_serie(library_id, serie_id.to_owned(), requesting_user).await?.ok_or(SourcesError::UnableToFindSerie(library_id.to_string(), serie_id.to_string(), "update_serie_image".to_string()))?;
        self.send_serie(SeriesMessage { library: library_id.to_string(), series: vec![SerieWithAction { serie, action: ElementAction::Updated}] });
        Ok(())
	}
    
}
