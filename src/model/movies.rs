use std::{collections::HashMap, io::Cursor};

use async_recursion::async_recursion;
use nanoid::nanoid;
use rs_plugin_common_interfaces::{
    domain::rs_ids::RsIds,
    lookup::{
        RsLookupMetadataResult, RsLookupMetadataResultWrapper, RsLookupMetadataResults,
        RsLookupMovie, RsLookupQuery,
    },
    ExternalImage, ImageType, MediaType,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;

use crate::{
    domain::{
        deleted::RsDeleted,
        library::LibraryRole,
        movie::{Movie, MovieExt, MovieForUpdate, MovieWithAction, MoviesMessage},
        ElementAction, MediaElement,
    },
    error::RsResult,
    plugins::{
        medias::imdb::ImdbContext,
        sources::{error::SourcesError, AsyncReadPinBox, FileStreamResult},
    },
    tools::image_tools::{convert_image_reader, ImageSize},
};

use super::{
    entity_images::EntityImageConfig,
    entity_search::merge_result_ids,
    error::{Error, Result},
    history::{history_id_rsids, movie_history_id},
    store::sql::SqlOrder,
    users::{ConnectedUser, HistoryQuery},
    ModelController,
};
use crate::routes::sse::SseEvent;

#[derive(
    Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display, EnumString, Default,
)]
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
    Custom(String),
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
        MovieQuery {
            after: None,
            ..Default::default()
        }
    }
    pub fn from_after(after: i64) -> MovieQuery {
        MovieQuery {
            after: Some(after),
            ..Default::default()
        }
    }
}

impl ModelController {
    async fn lookup_movie_metadata(
        &self,
        library_id: &str,
        query: RsLookupMovie,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<Movie>> {
        let lookup_ids = query.ids.clone().unwrap_or_default();
        let mut groups = self
            .exec_lookup_metadata_grouped(
                RsLookupQuery::Movie(query),
                Some(library_id.to_string()),
                requesting_user,
                None,
                None,
            )
            .await?;
        merge_result_ids(&mut groups);

        Ok(groups.into_iter().flat_map(|(_, _, r)| r.results).find_map(
            |result| match result.metadata {
                RsLookupMetadataResult::Movie(movie) => {
                    let result_ids: RsIds = movie.clone().into();
                    if lookup_ids.as_all_external_ids().is_empty()
                        || result_ids.has_common_id(&lookup_ids)
                    {
                        Some(movie)
                    } else {
                        None
                    }
                }
                _ => None,
            },
        ))
    }

    pub async fn get_movies(
        &self,
        library_id: &str,
        query: MovieQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Movie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let watched_query = query.watched;
        let mut movies = store.get_movies(query).await?;

        self.fill_movies_watched(&mut movies, requesting_user, Some(library_id.to_string()))
            .await?;
        if let Some(watched) = watched_query {
            movies.retain(|m| {
                if watched {
                    m.watched.is_some()
                } else {
                    m.watched.is_none()
                }
            });
        }
        Ok(movies)
    }

    pub async fn get_movie(
        &self,
        library_id: &str,
        movie_id: String,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;

        if RsIds::is_id(&movie_id) {
            let id: RsIds = movie_id.clone().try_into()?;
            let movie = store.get_movie_by_external_id(id.clone()).await?;
            if let Some(mut movie) = movie {
                self.fill_movie_watched(&mut movie, requesting_user, Some(library_id.to_string()))
                    .await?;
                Ok(movie)
            } else {
                let lookup_query = RsLookupMovie {
                    name: None,
                    ids: Some(id.clone()),
                    page_key: None,
                };
                if let Some(mut movie) = self
                    .lookup_movie_metadata(library_id, lookup_query, requesting_user)
                    .await?
                {
                    self.fill_movie_watched(
                        &mut movie,
                        requesting_user,
                        Some(library_id.to_string()),
                    )
                    .await?;
                    Ok(movie)
                } else {
                    Err(SourcesError::UnableToFindMovie(
                        library_id.to_string(),
                        movie_id.to_string(),
                        "get_movie".to_string(),
                    )
                    .into())
                }
            }
        } else {
            let mut movie =
                store
                    .get_movie(&movie_id)
                    .await?
                    .ok_or(SourcesError::UnableToFindMovie(
                        library_id.to_string(),
                        movie_id.to_string(),
                        "get_movie".to_string(),
                    ))?;
            self.fill_movie_watched(&mut movie, requesting_user, Some(library_id.to_string()))
                .await?;
            Ok(movie)
        }
    }

    pub async fn search_movie(
        &self,
        library_id: &str,
        query: RsLookupMovie,
        sources: Option<Vec<String>>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<(String, String, RsLookupMetadataResults)>> {
        self.search_entity(
            library_id,
            RsLookupQuery::Movie(query),
            |r| matches!(r.metadata, RsLookupMetadataResult::Movie(_)),
            None,
            sources,
            requesting_user,
        )
        .await
    }

    pub async fn search_movie_stream(
        &self,
        library_id: &str,
        query: RsLookupMovie,
        sources: Option<Vec<String>>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<tokio::sync::mpsc::Receiver<(String, String, RsLookupMetadataResults)>> {
        self.search_entity_stream(
            library_id,
            RsLookupQuery::Movie(query),
            |r| matches!(r.metadata, RsLookupMetadataResult::Movie(_)),
            None,
            sources,
            requesting_user,
        )
        .await
    }

    pub async fn fill_movie_watched(
        &self,
        movie: &mut Movie,
        requesting_user: &ConnectedUser,
        library_id: Option<String>,
    ) -> RsResult<()> {
        movie.fill_imdb_ratings(&self.imdb).await;
        let history_id = movie_history_id(movie);

        let progress = self
            .get_view_progress(
                history_id_rsids(history_id.clone()),
                requesting_user,
                library_id.clone(),
            )
            .await?;
        if let Some(progress) = progress {
            movie.progress = Some(progress.progress);
        }

        let watched = self
            .get_watched(
                HistoryQuery {
                    types: vec![MediaType::Movie],
                    id: Some(history_id_rsids(history_id)),
                    ..Default::default()
                },
                requesting_user,
                library_id,
            )
            .await?;
        let watched = watched.first();
        if let Some(watched) = watched {
            movie.watched = Some(watched.date);
        }
        Ok(())
    }
    pub async fn fill_movies_watched(
        &self,
        movies: &mut Vec<Movie>,
        requesting_user: &ConnectedUser,
        library_id: Option<String>,
    ) -> RsResult<()> {
        let progresses = self
            .get_all_view_progress(
                HistoryQuery {
                    types: vec![MediaType::Movie],
                    ..Default::default()
                },
                requesting_user,
                library_id.clone(),
            )
            .await?
            .into_iter()
            .map(|e| (e.id, e.progress))
            .collect::<HashMap<_, _>>();
        let watched = self
            .get_watched(
                HistoryQuery {
                    types: vec![MediaType::Movie],
                    ..Default::default()
                },
                requesting_user,
                library_id,
            )
            .await?
            .into_iter()
            .map(|e| (e.id, e.date))
            .collect::<HashMap<_, _>>();
        for movie in movies {
            let history_id = movie_history_id(movie);
            if let Some(watch) = watched.get(&history_id) {
                movie.watched = Some(*watch);
            }
            if let Some(progress) = progresses.get(&history_id) {
                movie.progress = Some(*progress);
            }

            movie.fill_imdb_ratings(&self.imdb).await;
        }
        Ok(())
    }

    pub async fn get_movie_by_external_id(
        &self,
        library_id: &str,
        ids: RsIds,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let movie = store.get_movie_by_external_id(ids.clone()).await?.ok_or(
            SourcesError::UnableToFindMovie(
                library_id.to_string(),
                format!("External: {:?}", ids),
                "get_movie_by_external_id".to_string(),
            ),
        )?;
        Ok(movie)
    }

    pub async fn get_movie_ids(
        &self,
        library_id: &str,
        movie_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<RsIds> {
        let movie = self
            .get_movie(library_id, movie_id.to_string(), requesting_user)
            .await?;
        let ids: RsIds = movie.into();
        Ok(ids)
    }

    pub async fn trending_movies(&self, requesting_user: &ConnectedUser) -> RsResult<Vec<Movie>> {
        let _ = requesting_user;
        Err(crate::Error::NotImplemented(
            "movie trending must come from a metadata plugin".to_string(),
        ))
    }

    pub async fn update_movie(
        &self,
        library_id: &str,
        movie_id: String,
        update: MovieForUpdate,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        if RsIds::is_id(&movie_id) {
            return Err(Error::InvalidIdForAction("udpate".to_string(), movie_id).into());
        }
        if update.has_update() {
            let store = self.store.get_library_store(library_id)?;
            store.update_movie(&movie_id, update).await?;
            let person =
                store
                    .get_movie(&movie_id)
                    .await?
                    .ok_or(SourcesError::UnableToFindMovie(
                        library_id.to_string(),
                        movie_id.to_string(),
                        "update_movie".to_string(),
                    ))?;
            self.send_movie(MoviesMessage {
                library: library_id.to_string(),
                movies: vec![MovieWithAction {
                    action: ElementAction::Updated,
                    movie: person.clone(),
                }],
            });
            Ok(person)
        } else {
            let movie = self
                .get_movie(library_id, movie_id, requesting_user)
                .await?;
            Ok(movie)
        }
    }

    pub async fn refresh_movies_imdb(
        &self,
        library_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        let movies = self
            .get_movies(library_id, MovieQuery::default(), requesting_user)
            .await?;
        //Imdb rating
        for mut movie in movies {
            let existing_votes = movie.imdb_votes.unwrap_or(0);
            movie.fill_imdb_ratings(&self.imdb).await;
            if existing_votes != movie.imdb_votes.unwrap_or(0) {
                self.update_movie(
                    library_id,
                    movie.id,
                    MovieForUpdate {
                        imdb_rating: movie.imdb_rating,
                        imdb_votes: movie.imdb_votes,
                        ..Default::default()
                    },
                    &ConnectedUser::ServerAdmin,
                )
                .await?;
            }
        }
        Ok(())
    }

    pub fn send_movie(&self, message: MoviesMessage) {
        self.broadcast_sse(SseEvent::Movies(message));
    }

    pub async fn add_movie(
        &self,
        library_id: &str,
        mut new_movie: Movie,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let ids: RsIds = new_movie.clone().into();
        let existing = self
            .get_movie_by_external_id(library_id, ids, requesting_user)
            .await;
        if let Ok(existing) = existing {
            return Err(
                Error::Duplicate(existing.id.to_owned(), MediaElement::Movie(existing)).into(),
            );
        }
        let store = self.store.get_library_store(library_id)?;
        let id = nanoid!();
        new_movie.id = id.clone();
        store.add_movie(new_movie).await?;
        let new_person = self.get_movie(library_id, id, requesting_user).await?;
        self.send_movie(MoviesMessage {
            library: library_id.to_string(),
            movies: vec![MovieWithAction {
                action: ElementAction::Added,
                movie: new_person.clone(),
            }],
        });

        let mc = self.clone();
        let lib_id = library_id.to_string();
        let mid = new_person.id.clone();
        let user = requesting_user.clone();
        tokio::spawn(async move {
            let _ = mc.enrich_movie_ids(&lib_id, &mid, &user).await;
        });

        Ok(new_person)
    }

    pub async fn enrich_movie_ids(
        &self,
        library_id: &str,
        movie_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        let movie = self
            .get_movie(library_id, movie_id.to_string(), requesting_user)
            .await?;
        let ids: RsIds = movie.clone().into();
        if ids.as_all_external_ids().is_empty() {
            return Ok(());
        }

        let lookup_query = RsLookupQuery::Movie(RsLookupMovie {
            name: None,
            ids: Some(ids.clone()),
            page_key: None,
        });
        let mut groups = self
            .exec_lookup_metadata_grouped(
                lookup_query,
                Some(library_id.to_string()),
                requesting_user,
                None,
                None,
            )
            .await?;
        merge_result_ids(&mut groups);

        let matched = groups
            .into_iter()
            .flat_map(|(_, _, r)| r.results)
            .find_map(|result| {
                if let RsLookupMetadataResult::Movie(m) = result.metadata {
                    let result_ids: RsIds = m.clone().into();
                    if result_ids.has_common_id(&ids) {
                        Some(m)
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

        if let Some(matched) = matched {
            let mut updates = MovieForUpdate::default();
            if movie.imdb.is_none() {
                updates.imdb = matched.imdb;
            }
            if movie.tmdb.is_none() {
                updates.tmdb = matched.tmdb;
            }
            if movie.trakt.is_none() {
                updates.trakt = matched.trakt;
            }
            if movie.slug.is_none() {
                updates.slug = matched.slug;
            }
            if movie.year.is_none() {
                updates.year = matched.year.map(|y| y as u32);
            }
            if movie.overview.is_none() {
                updates.overview = matched.overview;
            }
            if movie.status.is_none() {
                updates.status = matched.status;
            }
            if updates.has_update() {
                self.update_movie(
                    library_id,
                    movie_id.to_string(),
                    updates,
                    &ConnectedUser::ServerAdmin,
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn remove_movie(
        &self,
        library_id: &str,
        movie_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if RsIds::is_id(movie_id) {
            return Err(
                Error::InvalidIdForAction("remove".to_string(), movie_id.to_string()).into(),
            );
        }
        let store = self.store.get_library_store(library_id)?;
        let existing = store
            .get_movie(movie_id)
            .await?
            .ok_or(SourcesError::UnableToFindMovie(
                library_id.to_string(),
                movie_id.to_string(),
                "update_movie".to_string(),
            ))?;

        store.remove_movie(movie_id.to_string()).await?;
        self.add_deleted(
            library_id,
            RsDeleted::movie(movie_id.to_owned()),
            requesting_user,
        )
        .await?;
        self.send_movie(MoviesMessage {
            library: library_id.to_string(),
            movies: vec![MovieWithAction {
                action: ElementAction::Deleted,
                movie: existing.clone(),
            }],
        });
        Ok(existing)
    }

    pub async fn import_movie(
        &self,
        library_id: &str,
        movie_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if let Ok(ids) = RsIds::try_from(movie_id.to_string()) {
            let existing = self
                .get_movie_by_external_id(library_id, ids.clone(), requesting_user)
                .await;
            if let Ok(existing) = existing {
                Err(Error::Duplicate(existing.id.to_owned(), MediaElement::Movie(existing)).into())
            } else {
                let lookup_query = RsLookupMovie {
                    name: None,
                    ids: Some(ids.clone()),
                    page_key: None,
                };
                let new_movie = if let Some(movie) = self
                    .lookup_movie_metadata(library_id, lookup_query, requesting_user)
                    .await?
                {
                    movie
                } else {
                    return Err(SourcesError::UnableToFindMovie(
                        library_id.to_string(),
                        movie_id.to_string(),
                        "import_movie".to_string(),
                    )
                    .into());
                };
                let imported_movie = self
                    .add_movie(library_id, new_movie, requesting_user)
                    .await?;
                Ok(imported_movie)
            }
        } else {
            Err(Error::InvalidIdForAction("import".to_string(), movie_id.to_string()).into())
        }
    }

    pub async fn refresh_movie(
        &self,
        library_id: &str,
        movie_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Movie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let ids = self
            .get_movie_ids(library_id, movie_id, requesting_user)
            .await?;
        let movie = self
            .get_movie(library_id, movie_id.to_string(), requesting_user)
            .await?;
        let lookup_query = RsLookupMovie {
            name: Some(movie.name.clone()),
            ids: Some(ids.clone()),
            page_key: None,
        };
        let new_movie = if let Some(movie) = self
            .lookup_movie_metadata(library_id, lookup_query, requesting_user)
            .await?
        {
            movie
        } else {
            return Err(SourcesError::UnableToFindMovie(
                library_id.to_string(),
                movie_id.to_string(),
                "refresh_movie".to_string(),
            )
            .into());
        };
        let mut updates = MovieForUpdate {
            ..Default::default()
        };

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

        let new_movie = self
            .update_movie(library_id, movie_id.to_string(), updates, requesting_user)
            .await?;
        Ok(new_movie)
    }

    #[async_recursion]
    pub async fn movie_image(
        &self,
        library_id: &str,
        movie_id: &str,
        kind: Option<ImageType>,
        size: Option<ImageSize>,
        requesting_user: &ConnectedUser,
    ) -> crate::Result<FileStreamResult<AsyncReadPinBox>> {
        let kind = kind.unwrap_or(ImageType::Poster);
        let config = EntityImageConfig {
            folder: ".movies",
            cache_prefix: "movie",
        };
        if RsIds::is_id(movie_id) {
            let movie_ids: RsIds = movie_id.to_string().try_into()?;
            let store = self.store.get_library_store(library_id)?;
            let existing_movie = store.get_movie_by_external_id(movie_ids.clone()).await?;
            if let Some(existing_movie) = existing_movie {
                return self
                    .movie_image(
                        library_id,
                        &existing_movie.id,
                        Some(kind),
                        size,
                        requesting_user,
                    )
                    .await;
            }
            let lookup_query = RsLookupQuery::Movie(RsLookupMovie {
                name: None,
                ids: Some(movie_ids),
                page_key: None,
            });
            self.serve_cached_entity_image(
                library_id,
                movie_id,
                lookup_query,
                &kind,
                &config,
                requesting_user,
            )
            .await
        } else {
            self.serve_local_entity_image(
                library_id,
                movie_id,
                &kind,
                size,
                &config,
                requesting_user,
                self.refresh_movie_image(library_id, movie_id, &kind, requesting_user),
            )
            .await
        }
    }

    pub async fn get_movie_images(
        &self,
        query: RsLookupMovie,
        library_id: Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<ExternalImage>> {
        self.get_entity_images(RsLookupQuery::Movie(query), library_id, requesting_user)
            .await
    }

    pub async fn refresh_movie_image(
        &self,
        library_id: &str,
        movie_id: &str,
        kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        let movie = self
            .get_movie(library_id, movie_id.to_string(), requesting_user)
            .await?;
        let ids: RsIds = movie.clone().into();
        let lookup_query = RsLookupQuery::Movie(RsLookupMovie {
            name: Some(movie.name.clone()),
            ids: Some(ids),
            page_key: None,
        });
        let reader = self
            .download_entity_image(
                lookup_query,
                Some(library_id.to_string()),
                kind,
                requesting_user,
            )
            .await?;
        self.update_movie_image(
            library_id,
            movie_id,
            kind,
            reader,
            &ConnectedUser::ServerAdmin,
        )
        .await?;
        Ok(())
    }

    pub async fn get_movie_image_url(
        &self,
        query: RsLookupMovie,
        library_id: Option<String>,
        kind: &ImageType,
        _lang: &Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<rs_plugin_common_interfaces::RsRequest>> {
        self.get_entity_image_url(
            RsLookupQuery::Movie(query),
            library_id,
            kind,
            requesting_user,
        )
        .await
    }

    pub async fn download_movie_image(
        &self,
        query: RsLookupMovie,
        library_id: Option<String>,
        kind: &ImageType,
        _lang: &Option<String>,
        requesting_user: &ConnectedUser,
    ) -> crate::Result<AsyncReadPinBox> {
        self.download_entity_image(
            RsLookupQuery::Movie(query),
            library_id,
            kind,
            requesting_user,
        )
        .await
    }

    pub async fn update_movie_image(
        &self,
        library_id: &str,
        movie_id: &str,
        kind: &ImageType,
        reader: AsyncReadPinBox,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if RsIds::is_id(movie_id) {
            return Err(Error::InvalidIdForAction(
                "udpate movie image".to_string(),
                movie_id.to_string(),
            )
            .into());
        }

        let converted =
            convert_image_reader(reader, image::ImageFormat::Avif, Some(60), false).await?;
        let converted_reader = Cursor::new(converted);

        self.update_library_image(
            library_id,
            ".movies",
            movie_id,
            &Some(kind.clone()),
            &None,
            converted_reader,
            requesting_user,
        )
        .await?;

        let store = self.store.get_library_store(library_id)?;
        store
            .update_movie_image(movie_id.to_string(), kind.clone())
            .await;

        let movie = self
            .get_movie(library_id, movie_id.to_owned(), requesting_user)
            .await?;
        self.send_movie(MoviesMessage {
            library: library_id.to_string(),
            movies: vec![MovieWithAction {
                movie,
                action: ElementAction::Updated,
            }],
        });
        Ok(())
    }
}
