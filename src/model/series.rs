use std::{collections::HashMap, io::Cursor};

use async_recursion::async_recursion;
use nanoid::nanoid;
use rs_plugin_common_interfaces::{
    ExternalImage,
    ImageType,
    domain::{rs_ids::RsIds, serie::SerieStatus, ItemWithRelations},
    lookup::{RsLookupMetadataResult, RsLookupMetadataResultWrapper, RsLookupMetadataResults, RsLookupMovie, RsLookupQuery, RsLookupSerie},
};
use rusqlite::{
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    ToSql,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    domain::{
        ElementAction,
        MediaElement,
        deleted::RsDeleted,
        episode::EpisodeExt,
        library::{LibraryRole, LibraryType},
        serie::{Serie, SerieExt, SerieWithAction, SeriesMessage},
    },
    error::RsResult,
    plugins::{
        medias::imdb::ImdbContext,
        sources::{
            AsyncReadPinBox, FileStreamResult, error::SourcesError,
        },
    },
    tools::image_tools::{convert_image_reader, ImageSize},
};

use super::{
    entity_images::EntityImageConfig,
    episodes::{EpisodeForUpdate, EpisodeQuery},
    error::{Error, Result},
    medias::{MediaQuery, RsSort},
    store::sql::SqlOrder,
    users::ConnectedUser,
    ModelController,
};
use crate::routes::sse::SseEvent;


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
        SerieQuery {
            after: None,
            ..Default::default()
        }
    }
    pub fn from_after(after: i64) -> SerieQuery {
        SerieQuery {
            after: Some(after),
            ..Default::default()
        }
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
    pub openlibrary_work_id: Option<String>,
    pub anilist_manga_id: Option<u64>,
    pub mangadex_manga_uuid: Option<String>,
    pub myanimelist_manga_id: Option<u64>,

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

impl ModelController {
    pub async fn get_series(
        &self,
        library_id: &str,
        query: SerieQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<ItemWithRelations<Serie>>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let people = store.get_series(query).await?;
        Ok(people)
    }

    pub async fn get_serie(
        &self,
        library_id: &str,
        serie_id: String,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<ItemWithRelations<Serie>>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;

        if RsIds::is_id(&serie_id) {
            let id: RsIds = serie_id.clone().try_into().map_err(|_| {
                SourcesError::UnableToFindSerie(
                    library_id.to_string(),
                    format!("{:?}", serie_id),
                    "get_serie".to_string(),
                )
            })?;
            let serie = store.get_serie_by_external_id(id.clone()).await?;
            if let Some(serie) = serie {
                Ok(Some(serie))
            } else {
                // Try plugin lookup first
                let lookup_query = RsLookupQuery::Serie(RsLookupSerie {
                    name: Some(String::new()),
                    ids: Some(id.clone()),
                    page_key: None,
                });
                let plugin_results = self
                    .exec_lookup_metadata_grouped(
                        lookup_query,
                        Some(library_id.to_string()),
                        requesting_user,
                        None,
                        None,
                    )
                    .await?;
                let plugin_serie = plugin_results
                    .into_iter()
                    .flat_map(|(_, _, r)| r.results)
                    .find_map(|result| match result.metadata {
                        RsLookupMetadataResult::Serie(serie) => Some(serie),
                        _ => None,
                    });
                if let Some(serie) = plugin_serie {
                    return Ok(Some(ItemWithRelations { item: serie, relations: None }));
                }

                // Fallback to Trakt
                let mut trakt_show = self.trakt.get_serie(&id).await.map_err(|_| {
                    SourcesError::UnableToFindSerie(
                        library_id.to_string(),
                        format!("{:?}", id),
                        "get_serie".to_string(),
                    )
                })?;
                trakt_show.fill_imdb_ratings(&self.imdb).await;
                Ok(Some(ItemWithRelations { item: trakt_show, relations: None }))
            }
        } else {
            let serie = store.get_serie(&serie_id).await?;
            Ok(serie)
        }
    }

    pub async fn get_serie_by_external_id(
        &self,
        library_id: &str,
        ids: RsIds,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<ItemWithRelations<Serie>>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let serie = store.get_serie_by_external_id(ids).await?;
        Ok(serie)
    }

    /// Find a serie in the library by matching its local ID, any external identifier, or name/alt.
    pub async fn get_serie_by_any_id(
        &self,
        library_id: &str,
        serie: &Serie,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<Serie>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;

        // 1. Exact local ID
        if let Some(found) = store.get_serie(&serie.id).await? {
            return Ok(Some(found.item));
        }

        // 2. Any external ID
        let ids: RsIds = serie.clone().into();
        if let Some(found) = store.get_serie_by_external_id(ids).await? {
            return Ok(Some(found.item));
        }

        // 3. Name / alt fallback
        let mut names = vec![serie.name.clone()];
        if let Some(alts) = &serie.alt {
            names.extend(alts.clone());
        }
        for name in &names {
            if let Some(found) = store
                .get_series(SerieQuery { name: Some(name.clone()), ..Default::default() })
                .await?
                .into_iter()
                .next()
            {
                return Ok(Some(found.item));
            }
        }

        Ok(None)
    }

    pub async fn get_serie_ids(
        &self,
        library_id: &str,
        serie_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<RsIds> {
        let serie = self
            .get_serie(library_id, serie_id.to_string(), requesting_user)
            .await?
            .ok_or(Error::LibraryStoreNotFoundFor(
                library_id.to_string(),
                "get_serie_ids".to_string(),
            ))?;
        let ids: RsIds = serie.item.into();
        Ok(ids)
    }

    pub async fn trending_shows(&self) -> RsResult<Vec<Serie>> {
        self.trakt.trending_shows().await
    }

    pub async fn search_serie(
        &self,
        library_id: &str,
        query: RsLookupMovie,
        sources: Option<Vec<String>>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<(String, String, RsLookupMetadataResults)>> {
        let is_books_library = self.is_books_library(library_id).await;
        let include_trakt = !is_books_library && sources.as_deref().map_or(true, |s| s.iter().any(|id| id == "trakt"));
        let trakt_entries = if include_trakt {
            let trakt_results = self.trakt.search_show(&query).await?;
            Some(trakt_results.into_iter().map(|(serie, match_type)| RsLookupMetadataResultWrapper {
                metadata: RsLookupMetadataResult::Serie(serie),
                match_type,
                ..Default::default()
            }).collect())
        } else {
            None
        };

        let lookup_query = RsLookupQuery::Serie(RsLookupSerie {
            name: query.name,
            ids: query.ids,
            page_key: query.page_key,
        });
        self.search_entity(
            library_id,
            lookup_query,
            |r| matches!(r.metadata, RsLookupMetadataResult::Serie(_)),
            trakt_entries,
            sources,
            requesting_user,
        ).await
    }

    pub async fn search_serie_stream(
        &self,
        library_id: &str,
        query: RsLookupMovie,
        sources: Option<Vec<String>>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<tokio::sync::mpsc::Receiver<(String, String, RsLookupMetadataResults)>> {
        let is_books_library = self.is_books_library(library_id).await;
        let include_trakt = !is_books_library && sources.as_deref().map_or(true, |s| s.iter().any(|id| id == "trakt"));
        let trakt_entries = if include_trakt {
            let trakt_results = self.trakt.search_show(&query).await?;
            Some(trakt_results.into_iter().map(|(serie, match_type)| RsLookupMetadataResultWrapper {
                metadata: RsLookupMetadataResult::Serie(serie),
                match_type,
                ..Default::default()
            }).collect())
        } else {
            None
        };

        let lookup_query = RsLookupQuery::Serie(RsLookupSerie {
            name: query.name,
            ids: query.ids,
            page_key: query.page_key,
        });
        self.search_entity_stream(
            library_id,
            lookup_query,
            |r| matches!(r.metadata, RsLookupMetadataResult::Serie(_)),
            trakt_entries,
            sources,
            requesting_user,
        ).await
    }

    async fn is_books_library(&self, library_id: &str) -> bool {
        if let Some(library) = self.cache_get_library(library_id).await {
            library.kind == LibraryType::Books
        } else {
            self.get_internal_library(library_id)
                .await
                .ok()
                .flatten()
                .map(|library| library.kind == LibraryType::Books)
                .unwrap_or(false)
        }
    }

    pub async fn update_serie(
        &self,
        library_id: &str,
        serie_id: String,
        update: SerieForUpdate,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        if RsIds::is_id(&serie_id) {
            return Err(Error::InvalidIdForAction("udpate".to_string(), serie_id).into());
        }
        if update.has_update() {
            let store = self.store.get_library_store(library_id)?;
            store.update_serie(&serie_id, update).await?;
            let serie =
                store
                    .get_serie(&serie_id)
                    .await?
                    .ok_or(SourcesError::UnableToFindSerie(
                        library_id.to_string(),
                        serie_id,
                        "get_serie".to_string(),
                    ))?
                    .item;
            self.send_serie(SeriesMessage {
                library: library_id.to_string(),
                series: vec![SerieWithAction {
                    action: ElementAction::Updated,
                    serie: serie.clone(),
                }],
            });
            Ok(serie)
        } else {
            let serie = self
                .get_serie(library_id, serie_id.clone(), requesting_user)
                .await?
                .ok_or(SourcesError::UnableToFindSerie(
                    library_id.to_string(),
                    serie_id,
                    "get_serie".to_string(),
                ))?
                .item;
            Ok(serie)
        }
    }

    pub fn send_serie(&self, message: SeriesMessage) {
        self.broadcast_sse(SseEvent::Series(message));
    }

    pub async fn add_serie(
        &self,
        library_id: &str,
        mut new_serie: Serie,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let ids: RsIds = new_serie.clone().into();
        let existing = self
            .get_serie_by_external_id(library_id, ids, requesting_user)
            .await?;
        if let Some(existing) = existing {
            return Err(
                Error::Duplicate(existing.item.id.to_owned(), MediaElement::Serie(existing.item)).into(),
            );
        }
        let store = self.store.get_library_store(library_id)?;
        let id = nanoid!();
        new_serie.id = id.clone();
        store.add_serie(new_serie).await?;
        let inserted_serie = self
            .get_serie(library_id, id.clone(), requesting_user)
            .await?
            .ok_or(SourcesError::UnableToFindSerie(
                library_id.to_string(),
                id,
                "add_serie".to_string(),
            ))?
            .item;
        self.send_serie(SeriesMessage {
            library: library_id.to_string(),
            series: vec![SerieWithAction {
                action: ElementAction::Added,
                serie: inserted_serie.clone(),
            }],
        });

        let mc = self.clone();
        let inserted_serie_id = inserted_serie.id.clone();
        let library_id = library_id.to_string();
        let requesting_user = requesting_user.clone();
        tokio::spawn(async move {
            mc.refresh_serie(&library_id, &inserted_serie_id, &requesting_user)
                .await
                .unwrap();
            mc.refresh_episodes(&library_id, &inserted_serie_id, &requesting_user)
                .await
                .unwrap();
        });
        Ok(inserted_serie)
    }

    pub async fn remove_serie(
        &self,
        library_id: &str,
        serie_id: &str,
        delete_medias: bool,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if RsIds::is_id(serie_id) {
            return Err(
                Error::InvalidIdForAction("remove".to_string(), serie_id.to_string()).into(),
            );
        }
        let store = self.store.get_library_store(library_id)?;
        let existing = store
            .get_serie(serie_id)
            .await?
            .ok_or(SourcesError::UnableToFindSerie(
                library_id.to_string(),
                serie_id.to_string(),
                "remove_serie".to_string(),
            ))?
            .item;

        if delete_medias {
            let medias = self
                .get_medias(
                    library_id,
                    MediaQuery {
                        series: vec![existing.id.clone()],
                        ..Default::default()
                    },
                    requesting_user,
                )
                .await?;
            for media in medias {
                self.remove_media(library_id, &media.item.id, requesting_user)
                    .await?;
            }
        }

        store.remove_serie(serie_id.to_string()).await?;
        self.add_deleted(
            library_id,
            RsDeleted::serie(serie_id.to_owned()),
            requesting_user,
        )
        .await?;
        self.send_serie(SeriesMessage {
            library: library_id.to_string(),
            series: vec![SerieWithAction {
                action: ElementAction::Deleted,
                serie: existing.clone(),
            }],
        });
        Ok(existing)
    }

    pub async fn refresh_serie(
        &self,
        library_id: &str,
        serie_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let ids = self
            .get_serie_ids(library_id, serie_id, requesting_user)
            .await?;
        let serie = self
            .get_serie(library_id, serie_id.to_string(), requesting_user)
            .await?
            .ok_or(SourcesError::UnableToFindSerie(
                library_id.to_string(),
                serie_id.to_string(),
                "remove_serie".to_string(),
            ))?
            .item;
        let new_serie = self.trakt.get_serie(&ids).await?;
        let mut updates = SerieForUpdate {
            ..Default::default()
        };

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

        let new_serie = self
            .update_serie(library_id, serie_id.to_string(), updates, requesting_user)
            .await?;
        Ok(new_serie)
    }

    pub async fn refresh_series_imdb(
        &self,
        library_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        let all_series: Vec<Serie> = self
            .get_series(&library_id, SerieQuery::default(), &requesting_user)
            .await?
            .into_iter()
            .map(|iwr| iwr.item)
            .collect();
        //Imdb rating
        for mut serie in all_series {
            let existing_votes = serie.imdb_votes.unwrap_or(0);
            serie.fill_imdb_ratings(&self.imdb).await;
            let serieid = serie.id.clone();
            if existing_votes != serie.imdb_votes.unwrap_or(0) {
                self.update_serie(
                    library_id,
                    serie.id,
                    SerieForUpdate {
                        imdb_rating: serie.imdb_rating,
                        imdb_votes: serie.imdb_votes,
                        ..Default::default()
                    },
                    &ConnectedUser::ServerAdmin,
                )
                .await?;
            }
            let episodes = self
                .get_episodes(
                    library_id,
                    EpisodeQuery {
                        serie_ref: Some(serieid.clone()),
                        ..Default::default()
                    },
                    &ConnectedUser::ServerAdmin,
                )
                .await?;
            for mut episode in episodes {
                let existing_votes = episode.imdb_votes.unwrap_or(0);
                episode.fill_imdb_ratings(&self.imdb).await;
                if existing_votes != episode.imdb_votes.unwrap_or(0) {
                    self.update_episode(
                        library_id,
                        serieid.clone(),
                        episode.season,
                        episode.number,
                        EpisodeForUpdate {
                            imdb_rating: serie.imdb_rating,
                            imdb_votes: serie.imdb_votes,
                            ..Default::default()
                        },
                        &ConnectedUser::ServerAdmin,
                    )
                    .await?;
                }
            }
        }
        Ok(())
    }

    pub async fn import_serie(
        &self,
        library_id: &str,
        serie_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Serie> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if let Ok(ids) = RsIds::try_from(serie_id.to_string()) {
            let existing = self
                .get_serie_by_external_id(library_id, ids.clone(), requesting_user)
                .await?;
            if let Some(existing) = existing {
                Err(Error::Duplicate(existing.item.id.to_owned(), MediaElement::Serie(existing.item)).into())
            } else {
                let mut new_serie = self.trakt.get_serie(&ids).await?;
                new_serie.fill_imdb_ratings(&self.imdb).await;
                let imported_serie = self
                    .add_serie(library_id, new_serie, requesting_user)
                    .await?;
                Ok(imported_serie)
            }
        } else {
            Err(Error::InvalidIdForAction("import".to_string(), serie_id.to_string()).into())
        }
    }

    #[async_recursion]
    pub async fn serie_image(
        &self,
        library_id: &str,
        serie_id: &str,
        kind: Option<ImageType>,
        size: Option<ImageSize>,
        requesting_user: &ConnectedUser,
    ) -> crate::Result<FileStreamResult<AsyncReadPinBox>> {
        let kind = kind.unwrap_or(ImageType::Poster);
        let config = EntityImageConfig { folder: ".series", cache_prefix: "serie" };
        if RsIds::is_id(serie_id) {
            let mut serie_ids: RsIds = serie_id.to_string().try_into()?;
            let store = self.store.get_library_store(library_id)?;
            let existing_serie = store.get_serie_by_external_id(serie_ids.clone()).await?;
            if let Some(existing_serie) = existing_serie {
                return self.serie_image(library_id, &existing_serie.item.id, Some(kind), size, requesting_user).await;
            }
            // Enrich IDs via Trakt if needed
            let mut lookup_name = String::new();
            if serie_ids.tmdb().is_none() {
                if let Ok(serie) = self.trakt.get_serie(&serie_ids).await {
                    lookup_name = serie.name.clone();
                    serie_ids = serie.into();
                }
            }
            let lookup_query = RsLookupQuery::Serie(RsLookupSerie {
                name: if lookup_name.is_empty() { None } else { Some(lookup_name) },
                ids: Some(serie_ids),
                page_key: None,
            });
            self.serve_cached_entity_image(library_id, serie_id, lookup_query, &kind, &config, requesting_user).await
        } else {
            self.serve_local_entity_image(
                library_id, serie_id, &kind, size, &config, requesting_user,
                self.refresh_serie_image(library_id, serie_id, &kind, requesting_user),
            ).await
        }
    }

    pub async fn refresh_serie_image(
        &self,
        library_id: &str,
        serie_id: &str,
        kind: &ImageType,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        let serie = self
            .get_serie(library_id, serie_id.to_string(), requesting_user)
            .await?
            .ok_or(SourcesError::UnableToFindSerie(
                library_id.to_string(),
                serie_id.to_string(),
                "refresh_serie_image".to_string(),
            ))?;
        let serie_name = serie.item.name.clone();
        let ids: RsIds = serie.item.into();
        let lookup_query = RsLookupQuery::Serie(RsLookupSerie {
            name: Some(serie_name),
            ids: Some(ids),
            page_key: None,
        });
        let reader = self
            .download_entity_image(lookup_query, Some(library_id.to_string()), kind, requesting_user)
            .await?;
        self.update_serie_image(library_id, serie_id, kind, reader, &ConnectedUser::ServerAdmin).await?;
        Ok(())
    }

    pub async fn get_serie_image_url(
        &self,
        query: RsLookupSerie,
        library_id: Option<String>,
        kind: &ImageType,
        _lang: &Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<rs_plugin_common_interfaces::RsRequest>> {
        self.get_entity_image_url(RsLookupQuery::Serie(query), library_id, kind, requesting_user).await
    }

    pub async fn get_serie_images(
        &self,
        query: RsLookupSerie,
        library_id: Option<String>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<ExternalImage>> {
        self.get_entity_images(RsLookupQuery::Serie(query), library_id, requesting_user).await
    }

    pub async fn download_serie_image(
        &self,
        query: RsLookupSerie,
        library_id: Option<String>,
        kind: &ImageType,
        _lang: &Option<String>,
        requesting_user: &ConnectedUser,
    ) -> crate::Result<AsyncReadPinBox> {
        self.download_entity_image(RsLookupQuery::Serie(query), library_id, kind, requesting_user).await
    }

    pub async fn update_serie_image(
        &self,
        library_id: &str,
        serie_id: &str,
        kind: &ImageType,
        mut reader: AsyncReadPinBox,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        if RsIds::is_id(serie_id) {
            return Err(
                Error::InvalidIdForAction("udpate image".to_string(), serie_id.to_string()).into(),
            );
        }

        let converted =
            convert_image_reader(reader, image::ImageFormat::Avif, Some(60), false).await?;
        let converted_reader = Cursor::new(converted);

        self.update_library_image(
            library_id,
            ".series",
            serie_id,
            &Some(kind.clone()),
            &None,
            converted_reader,
            requesting_user,
        )
        .await?;

        let store = self.store.get_library_store(library_id)?;
        store
            .update_serie_image(serie_id.to_string(), kind.clone())
            .await;

        let serie = self
            .get_serie(library_id, serie_id.to_owned(), requesting_user)
            .await?
            .ok_or(SourcesError::UnableToFindSerie(
                library_id.to_string(),
                serie_id.to_string(),
                "update_serie_image".to_string(),
            ))?
            .item;
        self.send_serie(SeriesMessage {
            library: library_id.to_string(),
            series: vec![SerieWithAction {
                serie,
                action: ElementAction::Updated,
            }],
        });
        Ok(())
    }
}
