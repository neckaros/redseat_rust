


use std::{collections::HashMap, io::{self, Cursor, Read}, pin::Pin};

use async_recursion::async_recursion;
use futures::TryStreamExt;
use nanoid::nanoid;
use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, lookup::RsLookupMovie, ExternalImage, ImageType, MediaType};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;
use tokio::{fs::File, io::{AsyncRead, AsyncWriteExt, BufReader}};
use tokio_util::io::StreamReader;


use crate::{domain::{deleted::RsDeleted, library::LibraryRole, movie::{Movie, MovieForUpdate, MovieWithAction, MoviesMessage}, people::{PeopleMessage, Person}, ElementAction, MediaElement}, error::RsResult, plugins::{medias::imdb::ImdbContext, sources::{error::SourcesError, path_provider::PathProvider, AsyncReadPinBox, FileStreamResult, Source}}, server::get_server_folder_path_array, tools::{image_tools::{convert_image_reader, resize_image_reader, ImageSize}, log::log_info}};

use super::{error::{Error, Result}, store::sql::SqlOrder, users::{ConnectedUser, HistoryQuery}, ModelController};



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display,EnumString, Default)]
#[strum(serialize_all = "camelCase")]
#[serde(rename_all = "camelCase")]
#[serde(untagged)]
pub enum RsMovieSort {

    Modified,
    Added,
    Created,
    #[default]
    Name,
    Digitalairdate,
    #[strum(default)]
    Custom(String)
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct MovieQuery {
    pub after: Option<i64>,
    pub in_digital: Option<bool>,

    pub watched: Option<bool>,

    #[serde(default)]
    pub sort: RsMovieSort,
    pub order: Option<SqlOrder>,
}


impl MovieQuery {
    pub fn new_empty() -> MovieQuery {
        MovieQuery { after: None, ..Default::default() }
    }
    pub fn from_after(after: i64) -> MovieQuery {
        MovieQuery { after: Some(after), ..Default::default() }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExternalMovieImages {
    pub backdrop: Option<String>,
    pub logo: Option<String>,
    pub poster: Option<String>,
    pub still: Option<String>,
}

impl ExternalMovieImages {
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

impl Movie {
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

	pub async fn get_movies(&self, library_id: &str, query: MovieQuery, requesting_user: &ConnectedUser) -> RsResult<Vec<Movie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let watched_query = query.watched;
		let mut movies = store.get_movies(query).await?;

        self.fill_movies_watched(&mut movies, requesting_user, Some(library_id.to_string())).await?;
        if let Some(watched) = watched_query {
            movies.retain(|m| if watched { m.watched.is_some() } else { m.watched.is_none() });
        }
		Ok(movies)
	}

    pub async fn get_movie(&self, library_id: &str, movie_id: String, requesting_user: &ConnectedUser) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;

        if RsIds::is_id(&movie_id) {
            let id: RsIds = movie_id.try_into()?;
            let movie = store.get_movie_by_external_id(id.clone()).await?;
            if let Some(mut movie) = movie {
                self.fill_movie_watched(&mut movie, requesting_user, Some(library_id.to_string())).await?;
                Ok(movie)
            } else {
                let mut trakt_movie = self.trakt.get_movie(&id).await?;
                self.fill_movie_watched(&mut trakt_movie, requesting_user, Some(library_id.to_string())).await?;
                Ok(trakt_movie)
            }
        } else {
            let mut movie = store.get_movie(&movie_id).await?.ok_or(SourcesError::UnableToFindMovie(library_id.to_string(), movie_id.to_string(), "get_movie".to_string()))?;
            self.fill_movie_watched(&mut movie, requesting_user, Some(library_id.to_string())).await?;
            Ok(movie)
        }
	}

    
    pub async fn search_movie(&self, library_id: &str, query: RsLookupMovie, requesting_user: &ConnectedUser) -> RsResult<Vec<Movie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let searched = self.trakt.search_movie(&query).await?;
		Ok(searched)
	}

    pub async fn fill_movie_watched(&self, movie: &mut Movie, requesting_user: &ConnectedUser, library_id: Option<String>) -> RsResult<()> {
        movie.fill_imdb_ratings(&self.imdb).await;

        let ids: RsIds = movie.clone().into();

        let progress = self.get_view_progress(ids, requesting_user, library_id.clone()).await?;
        if let Some(progress) = progress {
            movie.progress = Some(progress.progress);
        }

        let watched = self.get_watched(HistoryQuery { types: vec![MediaType::Movie], id: Some(movie.clone().into()), ..Default::default() }, requesting_user, library_id).await?;
        let watched = watched.first();
        if let Some(watched) = watched {
            movie.watched = Some(watched.date);
        }
        Ok(())
    }
    pub async fn fill_movies_watched(&self, movies: &mut Vec<Movie>, requesting_user: &ConnectedUser, library_id: Option<String>) -> RsResult<()> {
        let progresses = self.get_all_view_progress(HistoryQuery { types: vec![MediaType::Movie], ..Default::default() }, requesting_user, library_id.clone()).await?.into_iter().map(|e| (e.id, e.progress)).collect::<HashMap<_, _>>();
        let watched = self.get_watched(HistoryQuery { types: vec![MediaType::Movie], ..Default::default() }, requesting_user, library_id).await?.into_iter().map(|e| (e.id, e.date)).collect::<HashMap<_, _>>();
        for movie in movies {
            let ids = RsIds::from(movie.clone());
            let ids_string: Vec<String> = ids.into();

            for id in ids_string {
                let watch = watched.get(&id);
                if let Some(watch) = watch {
                    movie.watched = Some(*watch);
                }
                let progress = progresses.get(&id);
                if let Some(progress) = progress {
                    movie.progress = Some(*progress);
                }
            }
            
            movie.fill_imdb_ratings(&self.imdb).await;
        }
        Ok(())
    }
    
    pub async fn get_movie_by_external_id(&self, library_id: &str, ids: RsIds, requesting_user: &ConnectedUser) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let movie = store.get_movie_by_external_id(ids.clone()).await?.ok_or(SourcesError::UnableToFindMovie(library_id.to_string(), format!("External: {:?}", ids), "get_movie_by_external_id".to_string()))?;
        Ok(movie)
    }


    pub async fn get_movie_ids(&self, library_id: &str, movie_id: &str, requesting_user: &ConnectedUser) -> RsResult<RsIds> {
        let movie = self.get_movie(library_id, movie_id.to_string(), requesting_user).await?;
        let ids: RsIds = movie.into();
        Ok(ids)
    }

    pub async fn trending_movies(&self, requesting_user: &ConnectedUser )  -> RsResult<Vec<Movie>> {
        let mut movies = self.trakt.trending_movies().await?;
        println!("GOT trending");
        self.fill_movies_watched(&mut movies, requesting_user, None).await?;
        Ok(movies)
    }




    pub async fn update_movie(&self, library_id: &str, movie_id: String, update: MovieForUpdate, requesting_user: &ConnectedUser) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        if RsIds::is_id(&movie_id) {
            return Err(Error::InvalidIdForAction("udpate".to_string(), movie_id).into())
        }
        if update.has_update() {
            let store = self.store.get_library_store(library_id)?;
            store.update_movie(&movie_id, update).await?;
            let person = store.get_movie(&movie_id).await?.ok_or(SourcesError::UnableToFindMovie(library_id.to_string(), movie_id.to_string(), "update_movie".to_string()))?;
            self.send_movie(MoviesMessage { library: library_id.to_string(), movies: vec![MovieWithAction {action: ElementAction::Updated, movie: person.clone()}] });
            Ok(person)
        } else {
            let movie = self.get_movie(library_id, movie_id, requesting_user).await?;
            Ok(movie)
        }  
	}

    pub async fn refresh_movies_imdb(&self, library_id: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
        let movies = self.get_movies(library_id, MovieQuery::default(), requesting_user).await?;
        //Imdb rating
        for mut movie in movies {
            let existing_votes = movie.imdb_votes.unwrap_or(0);
            movie.fill_imdb_ratings(&self.imdb).await;
            if existing_votes != movie.imdb_votes.unwrap_or(0) {
                self.update_movie(library_id, movie.id, MovieForUpdate { imdb_rating: movie.imdb_rating, imdb_votes: movie.imdb_votes, ..Default::default()}, &ConnectedUser::ServerAdmin).await?;
            }
           
        }
        Ok(())
    }



	pub fn send_movie(&self, message: MoviesMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("movies", message);
			}
		});
	}


    pub async fn add_movie(&self, library_id: &str, mut new_movie: Movie, requesting_user: &ConnectedUser) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;        
        let ids: RsIds = new_movie.clone().into();
        let existing = self.get_movie_by_external_id(library_id, ids, requesting_user).await;
        if let Ok(existing) = existing {
            return Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Movie(existing)).into())
        }
        let store = self.store.get_library_store(library_id)?;
        let id = nanoid!();
        new_movie.id = id.clone();
		store.add_movie(new_movie).await?;
        let new_person = self.get_movie(library_id, id, requesting_user).await?;
        self.send_movie(MoviesMessage { library: library_id.to_string(), movies: vec![MovieWithAction {action: ElementAction::Added, movie: new_person.clone()}] });
		Ok(new_person)
	}


    pub async fn remove_movie(&self, library_id: &str, movie_id: &str, requesting_user: &ConnectedUser) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if RsIds::is_id(movie_id) {
            return Err(Error::InvalidIdForAction("remove".to_string(), movie_id.to_string()).into())
        }
        let store = self.store.get_library_store(library_id)?;
        let existing = store.get_movie(movie_id).await?.ok_or(SourcesError::UnableToFindMovie(library_id.to_string(), movie_id.to_string(), "update_movie".to_string()))?;
        
        store.remove_movie(movie_id.to_string()).await?;
        self.add_deleted(library_id, RsDeleted::movie(movie_id.to_owned()), requesting_user).await?;
        self.send_movie(MoviesMessage { library: library_id.to_string(), movies: vec![MovieWithAction {action: ElementAction::Deleted, movie: existing.clone()}] });
        Ok(existing)

	}



    pub async fn import_movie(&self, library_id: &str, movie_id: &str, requesting_user: &ConnectedUser) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if let Ok(ids) = RsIds::try_from(movie_id.to_string()) {
            let existing = self.get_movie_by_external_id(library_id, ids.clone(), requesting_user).await;
            if let Ok(existing) = existing {
                Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Movie(existing)).into())
            } else { 
                let new_movie = self.trakt.get_movie(&ids).await?;
                let imported_movie = self.add_movie(library_id, new_movie, requesting_user).await?;
                Ok(imported_movie)
            }
        } else {
            
            Err(Error::InvalidIdForAction("import".to_string(), movie_id.to_string()).into())
        }
    
	}

    pub async fn refresh_movie(&self, library_id: &str, movie_id: &str, requesting_user: &ConnectedUser) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let ids = self.get_movie_ids(library_id, movie_id, requesting_user).await?;
        let movie = self.get_movie(library_id, movie_id.to_string(), requesting_user).await?;
        let new_movie = self.trakt.get_movie(&ids).await?;
        let mut updates = MovieForUpdate {..Default::default()};

        if movie.status != new_movie.status {
            updates.status = new_movie.status;
        }
        if movie.trakt_rating != new_movie.trakt_rating {
            updates.trakt_rating = new_movie.trakt_rating;
        }
        if movie.trakt_votes != new_movie.trakt_votes {
            updates.trakt_votes = new_movie.trakt_votes;
        }
        if movie.trailer != new_movie.trailer {
            updates.trailer = new_movie.trailer;
        }
        if movie.imdb != new_movie.imdb {
            updates.imdb = new_movie.imdb;
        }
        if movie.tmdb != new_movie.tmdb {
            updates.tmdb = new_movie.tmdb;
        }
        if movie.digitalairdate != new_movie.digitalairdate {
            updates.digitalairdate = new_movie.digitalairdate;
        }
        if movie.airdate != new_movie.airdate {
            updates.airdate = new_movie.airdate;
        }

        let new_movie = self.update_movie(library_id, movie_id.to_string(), updates, requesting_user).await?;
        Ok(new_movie)        
	}

    #[async_recursion]
	pub async fn movie_image(&self, library_id: &str, movie_id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> crate::Result<FileStreamResult<AsyncReadPinBox>> {
        let kind = kind.unwrap_or(ImageType::Poster);
        if RsIds::is_id(movie_id) {
            let mut movie_ids: RsIds = movie_id.to_string().try_into()?;
            let store = self.store.get_library_store(library_id)?;
            let existing_movie = store.get_movie_by_external_id(movie_ids.clone()).await?;
            if let Some(existing_movie) = existing_movie {
                let image = self.movie_image(library_id, &existing_movie.id, Some(kind), size, requesting_user).await?;
                Ok(image)
            } else {

                let local_provider = self.library_source_for_library(library_id).await?;
                if movie_ids.tmdb.is_none() {
                    let movie = self.trakt.get_movie(&movie_ids).await?;
                    movie_ids = movie.into();
                }
                let image_path = format!("cache/movie-{}-{}.avif", movie_id.replace(':', "-"), kind);

                if !local_provider.exists(&image_path).await {
                    let images = self.get_movie_image_url(&movie_ids, &kind, &None).await?.ok_or(crate::Error::NotFound(format!("Unable to get movie image url: {:?}",movie_ids)))?;
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
            if !self.has_library_image(library_id, ".movies", movie_id, Some(kind.clone()), requesting_user).await? {
                log_info(crate::tools::log::LogServiceType::Source, format!("Updating movie image: {}", movie_id));
                self.refresh_movie_image(library_id, movie_id, &kind, requesting_user).await?;
            }
            
            let image = self.library_image(library_id, ".movies", movie_id, Some(kind), size, requesting_user).await?;
            Ok(image)
        }
	}

    /// fetch the plugins to get images for this movie
    pub async fn get_movie_images(&self, ids: &RsIds) -> RsResult<Vec<ExternalImage>> {
        let mut images = self.tmdb.movie_images(ids.clone()).await?;
       
        let mut fanart = self.fanart.movie_images(ids.clone()).await?;
        images.append(&mut fanart);
        Ok(images)
    }

    /// download and update image
    pub async fn refresh_movie_image(&self, library_id: &str, movie_id: &str, kind: &ImageType, requesting_user: &ConnectedUser) -> RsResult<()> {
        let movie = self.get_movie(library_id, movie_id.to_string(), requesting_user).await?;
        let ids: RsIds = movie.clone().into();
        let reader = self.download_movie_image(&ids, kind, &movie.lang).await?;
        self.update_movie_image(library_id, movie_id, kind, reader, &ConnectedUser::ServerAdmin).await?;
        Ok(())
	}

    pub async fn get_movie_image_url(&self, ids: &RsIds, kind: &ImageType, lang: &Option<String>) -> RsResult<Option<String>> {
        println!("Movie image ids {:?}", ids);
        let images = if kind == &ImageType::Card {
            None
        } else { 
            self.tmdb.movie_image(ids.clone(), lang).await?.into_kind(kind.clone())
        };
        if images.is_none() {
            let images = self.fanart.movie_image(ids.clone()).await?.into_kind(kind.clone());
            Ok(images)
        } else {
            Ok(images)
        }
    }


    pub async fn download_movie_image(&self, ids: &RsIds, kind: &ImageType, lang: &Option<String>) -> crate::Result<AsyncReadPinBox> {
        let images = self.get_movie_image_url(ids, kind, lang).await?.ok_or(crate::Error::NotFound(format!("Unable to download movie image url: {:?} kind: {:?}",ids, kind)))?;
        let image_reader = reqwest::get(images).await?;
        let stream = image_reader.bytes_stream();
        let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
        let body_reader = StreamReader::new(body_with_io_error);
        Ok(Box::pin(body_reader))
    }

    pub async fn update_movie_image(&self, library_id: &str, movie_id: &str, kind: &ImageType, reader: AsyncReadPinBox, requesting_user: &ConnectedUser) -> RsResult<()> {

        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if RsIds::is_id(movie_id) {
            return Err(Error::InvalidIdForAction("udpate movie image".to_string(), movie_id.to_string()).into())
        }

        let converted = convert_image_reader(reader, image::ImageFormat::Avif, Some(60), false).await?;
        let converted_reader = Cursor::new(converted);
        
        self.update_library_image(library_id, ".movies", movie_id, &Some(kind.clone()), converted_reader, requesting_user).await?;
        
        let store = self.store.get_library_store(library_id)?;
		store.update_movie_image(movie_id.to_string(), kind.clone()).await;

        let movie = self.get_movie(library_id, movie_id.to_owned(), requesting_user).await?;
        self.send_movie(MoviesMessage { library: library_id.to_string(), movies: vec![MovieWithAction { movie, action: ElementAction::Updated}] });
        Ok(())

	}
    
}
