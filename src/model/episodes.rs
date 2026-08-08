use std::collections::{HashMap, HashSet};

use async_recursion::async_recursion;
use nanoid::nanoid;
use query_external_ip::SourceError;
use rs_plugin_common_interfaces::{
    domain::rs_ids::{ApplyRsIds, RsIds},
    lookup::{RsLookupEpisode, RsLookupMetadataResult, RsLookupQuery},
    ImageType, MediaType,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncRead;

use crate::{
    domain::{
        deleted::RsDeleted,
        episode::{self, Episode, EpisodeExt, EpisodeWithAction, EpisodeWithShow, EpisodesMessage},
        library::LibraryRole,
        people::{PeopleMessage, Person},
        serie::{self, Serie, SeriesMessage},
        ElementAction,
    },
    error::RsResult,
    plugins::{
        medias::imdb::ImdbContext,
        sources::{error::SourcesError, AsyncReadPinBox, FileStreamResult},
    },
    tools::{array_tools::Dedup, clock::now, image_tools::ImageSize},
};

use super::{
    entity_search::merge_result_ids,
    error::{Error, Result},
    history::{episode_history_id, episode_history_ids},
    medias::{RsSort, RsSortOrder},
    store::sql::SqlOrder,
    users::{ConnectedUser, HistoryQuery},
    ModelController,
};
use crate::routes::sse::SseEvent;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeQuery {
    pub serie_ref: Option<String>,
    pub season: Option<u32>,

    #[serde(default)]
    pub not_seasons: Vec<u32>,

    pub after: Option<i64>,

    pub aired_before: Option<i64>,
    pub aired_after: Option<i64>,

    #[serde(default)]
    pub sorts: Vec<RsSortOrder>,

    pub limit: Option<u32>,
}

impl EpisodeQuery {
    pub fn new_empty() -> EpisodeQuery {
        EpisodeQuery {
            ..Default::default()
        }
    }
    pub fn from_after(after: i64) -> EpisodeQuery {
        EpisodeQuery {
            after: Some(after),
            ..Default::default()
        }
    }

    pub fn limit_or_default(&self) -> u32 {
        self.limit.unwrap_or(200)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeForUpdate {
    pub abs: Option<u32>,

    pub name: Option<String>,
    pub overview: Option<String>,
    pub alt: Option<Vec<String>>,
    pub add_alts: Option<Vec<String>>,
    pub remove_alts: Option<Vec<String>>,

    pub airdate: Option<i64>,
    pub duration: Option<u64>,

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
}

fn should_retry_episode_lookup_with_enriched_ids(
    original_ids: &RsIds,
    enriched_ids: &RsIds,
) -> bool {
    enriched_ids.as_all_external_ids() != original_ids.as_all_external_ids()
}

impl ModelController {
    async fn episode_series_by_ref(
        &self,
        library_id: &str,
        episodes: &[Episode],
    ) -> RsResult<HashMap<String, Serie>> {
        let store = self.store.get_library_store(library_id)?;
        let serie_refs: HashSet<String> = episodes.iter().map(|episode| episode.serie.clone()).collect();
        let mut series = HashMap::new();
        for serie_ref in serie_refs {
            if let Some(serie) = store.get_serie(&serie_ref).await? {
                series.insert(serie_ref, serie.item);
            }
        }
        Ok(series)
    }

    async fn lookup_episodes_metadata(
        &self,
        library_id: &str,
        serie_id: &str,
        ids: &RsIds,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Episode>> {
        let mut episodes = Vec::new();
        let mut empty_streak = 0;

        for season in 1..=100 {
            let mut groups = self
                .exec_lookup_metadata_grouped(
                    RsLookupQuery::Episode(RsLookupEpisode {
                        ids: Some(ids.clone()),
                        season,
                        number: None,
                        ..Default::default()
                    }),
                    Some(library_id.to_string()),
                    requesting_user,
                    None,
                    None,
                )
                .await?;
            merge_result_ids(&mut groups);

            let season_episodes: Vec<Episode> = groups
                .into_iter()
                .flat_map(|(_, _, r)| r.results)
                .filter_map(|result| match result.metadata {
                    RsLookupMetadataResult::Episode(mut episode) => {
                        episode.serie = serie_id.to_string();
                        Some(episode)
                    }
                    _ => None,
                })
                .collect();

            if season_episodes.is_empty() {
                empty_streak += 1;
                if empty_streak >= 2 && !episodes.is_empty() {
                    break;
                }
            } else {
                empty_streak = 0;
                episodes.extend(season_episodes);
            }
        }

        episodes.sort_by_key(|episode| (episode.season, episode.number));
        episodes.dedup_by(|a, b| a.season == b.season && a.number == b.number);
        Ok(episodes)
    }

    pub async fn get_episodes(
        &self,
        library_id: &str,
        query: EpisodeQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Episode>> {
        if let Some(serie_id) = &query.serie_ref {
            return self
                .get_episodes_by_id(library_id, serie_id.to_owned(), query, requesting_user)
                .await;
        }
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(
            Error::LibraryStoreNotFoundFor(library_id.to_string(), "get_episodes".to_string()),
        )?;
        let mut episodes = store.get_episodes(query).await?;

        self.fill_episodes_watched_imdb(
            &mut episodes,
            requesting_user,
            Some(library_id.to_string()),
        )
        .await?;
        Ok(episodes)
    }

    pub async fn get_episodes_by_id(
        &self,
        library_id: &str,
        serie_id: String,
        mut query: EpisodeQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Episode>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(
            Error::LibraryStoreNotFoundFor(
                library_id.to_string(),
                "get_episodes_by_id".to_string(),
            ),
        )?;
        let mut episodes = if RsIds::is_id(&serie_id) {
            let id: RsIds = serie_id.clone().try_into().map_err(|_| {
                Error::UnableToConvertToRsIds(
                    serie_id.clone().to_string(),
                    "get_episodes_by_id".to_string(),
                )
            })?;
            let serie = store.get_serie_by_external_id(id.clone()).await?;

            if let Some(serie) = serie {
                query.serie_ref = Some(serie.item.id);
                store.get_episodes(query).await?
            } else {
                let mut episodes = self
                    .lookup_episodes_metadata(library_id, &serie_id, &id, requesting_user)
                    .await?;
                if episodes.is_empty() {
                    if let Some(serie) = self
                        .get_serie(library_id, serie_id.clone(), requesting_user)
                        .await?
                    {
                        let enriched_ids: RsIds = serie.item.into();
                        if should_retry_episode_lookup_with_enriched_ids(&id, &enriched_ids) {
                            episodes = self
                                .lookup_episodes_metadata(
                                    library_id,
                                    &serie_id,
                                    &enriched_ids,
                                    requesting_user,
                                )
                                .await?;
                        }
                    }
                }
                if episodes.is_empty() {
                    return Err(SourcesError::NotFound(Some(format!(
                        "get_episodes_by_id - Unable to find episodes for {:?}",
                        id
                    )))
                    .into());
                }
                episodes
            }
        } else {
            store.get_episodes(query).await?
        };

        self.fill_episodes_watched_imdb(
            &mut episodes,
            requesting_user,
            Some(library_id.to_string()),
        )
        .await?;
        Ok(episodes)
    }

    pub async fn fill_episode_watched_imdb(
        &self,
        episode: &mut Episode,
        requesting_user: &ConnectedUser,
        library_id: Option<String>,
    ) -> RsResult<()> {
        if let Some(library_id) = library_id {
            let store = self.store.get_library_store(&library_id)?;
            if let Some(serie) = store.get_serie(&episode.serie).await? {
                let history_ids = episode_history_ids(&serie.item, episode);
                let watched = self
                    .get_watched(
                        HistoryQuery {
                            types: vec![MediaType::Episode],
                            id: Some(history_ids.clone()),
                            ..Default::default()
                        },
                        requesting_user,
                        Some(library_id.clone()),
                    )
                    .await?;
                let progress = self
                    .get_view_progress(
                        history_ids,
                        requesting_user,
                        Some(library_id),
                    )
                    .await?;
                if let Some(progress) = progress {
                    episode.progress = Some(progress.progress);
                }
                if let Some(watched) = watched.first() {
                    episode.watched = Some(watched.date);
                }
            }
        }
        episode.fill_imdb_ratings(&self.imdb).await;
        Ok(())
    }
    pub async fn fill_episodes_watched_imdb(
        &self,
        episodes: &mut Vec<Episode>,
        requesting_user: &ConnectedUser,
        library_id: Option<String>,
    ) -> RsResult<()> {
        let series_by_ref = if let Some(library_id) = library_id.as_deref() {
            self.episode_series_by_ref(library_id, episodes).await?
        } else {
            HashMap::new()
        };
        let watched = self
            .get_watched(
                HistoryQuery {
                    types: vec![MediaType::Episode],
                    ..Default::default()
                },
                requesting_user,
                library_id.clone(),
            )
            .await?
            .into_iter()
            .map(|e| (e.id, e.date))
            .collect::<HashMap<_, _>>();
        let progresses = self
            .get_all_view_progress(
                HistoryQuery {
                    types: vec![MediaType::Episode],
                    ..Default::default()
                },
                requesting_user,
                library_id,
            )
            .await?
            .into_iter()
            .map(|e| (e.id, e.progress))
            .collect::<HashMap<_, _>>();

        for episode in episodes {
            if let Some(serie) = series_by_ref.get(&episode.serie) {
                let history_ids = episode_history_ids(serie, episode).as_all_ids();
                if let Some(watch) = history_ids.iter().find_map(|id| watched.get(id)) {
                    episode.watched = Some(*watch);
                }
                if let Some(progress) = history_ids.iter().find_map(|id| progresses.get(id)) {
                    episode.progress = Some(*progress);
                }
            }
            episode.fill_imdb_ratings(&self.imdb).await;
        }
        Ok(())
    }

    pub async fn get_episodes_upcoming(
        &self,
        library_id: &str,
        query: EpisodeQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Episode>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(
            Error::LibraryStoreNotFoundFor(
                library_id.clone().to_string(),
                "get_episodes_upcoming".to_string(),
            ),
        )?;
        let mut episodes = store.get_episodes_upcoming(query).await?;
        self.fill_episodes_watched_imdb(
            &mut episodes,
            requesting_user,
            Some(library_id.to_string()),
        )
        .await?;
        Ok(episodes)
    }

    pub async fn get_episodes_ondeck(
        &self,
        library_id: &str,
        query: EpisodeQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Episode>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(
            Error::LibraryStoreNotFoundFor(
                library_id.clone().to_string(),
                "get_episodes_ondeck".to_string(),
            ),
        )?;
        let mut episodes = store.get_episodes_aired(query).await?;
        self.fill_episodes_watched_imdb(
            &mut episodes,
            requesting_user,
            Some(library_id.to_string()),
        )
        .await?;
        let mut episodes = episodes
            .into_iter()
            .filter(|e| e.watched.is_none())
            .collect::<Vec<_>>()
            .dedup_key(|e| e.serie.clone());
        episodes.reverse();
        Ok(episodes)
    }

    pub async fn get_episode(
        &self,
        library_id: &str,
        serie_id: String,
        season: u32,
        number: u32,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(
            Error::LibraryStoreNotFoundFor(serie_id.clone().to_string(), "get_episode".to_string()),
        )?;
        let mut episode = match store.get_episode(&serie_id, season, number).await? {
            Some(episode) => episode,
            None => store
                .get_episodes(EpisodeQuery {
                    serie_ref: Some(serie_id.clone()),
                    season: Some(season),
                    ..Default::default()
                })
                .await?
                .into_iter()
                .find(|episode| episode.number == number)
                .ok_or(SourcesError::UnableToFindEpisodes(
                    format!("{} {} {}", serie_id, season, number),
                    "get_episode".to_string(),
                ))?,
        };
        self.fill_episode_watched_imdb(&mut episode, requesting_user, Some(library_id.to_string()))
            .await?;
        Ok(episode)
    }

    pub async fn update_episode(
        &self,
        library_id: &str,
        serie_id: String,
        season: u32,
        number: u32,
        update: EpisodeForUpdate,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(
            Error::LibraryStoreNotFoundFor(
                library_id.clone().to_string(),
                "update_episode".to_string(),
            ),
        )?;
        store
            .update_episode(&serie_id, season, number, update)
            .await?;
        let episode = self
            .get_episode(library_id, serie_id, season, number, requesting_user)
            .await?;
        self.send_episode(EpisodesMessage {
            library: library_id.to_string(),
            episodes: vec![EpisodeWithAction {
                action: ElementAction::Updated,
                episode: episode.clone(),
            }],
        });
        Ok(episode)
    }

    pub fn send_episode(&self, message: EpisodesMessage) {
        self.broadcast_sse(SseEvent::Episodes(message));
    }

    pub async fn add_episode(
        &self,
        library_id: &str,
        new_serie: Episode,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(
            Error::LibraryStoreNotFoundFor(
                library_id.clone().to_string(),
                "add_episode".to_string(),
            ),
        )?;
        store.add_episode(new_serie.clone()).await?;
        let new_episode = self
            .get_episode(
                library_id,
                new_serie.serie,
                new_serie.season,
                new_serie.number,
                requesting_user,
            )
            .await?;
        self.send_episode(EpisodesMessage {
            library: library_id.to_string(),
            episodes: vec![EpisodeWithAction {
                action: ElementAction::Added,
                episode: new_episode.clone(),
            }],
        });
        Ok(new_episode)
    }

    pub async fn remove_episode(
        &self,
        library_id: &str,
        serie_id: &str,
        season: u32,
        number: u32,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Episode> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(
            Error::LibraryStoreNotFoundFor(
                library_id.clone().to_string(),
                "remove_episode".to_string(),
            ),
        )?;
        let existing = store.get_episode(serie_id, season, number).await?;
        if let Some(existing) = existing {
            store
                .remove_episode(serie_id.to_string(), season, number)
                .await?;
            self.add_deleted(
                library_id,
                RsDeleted::episode(existing.id()),
                requesting_user,
            )
            .await?;
            self.send_episode(EpisodesMessage {
                library: library_id.to_string(),
                episodes: vec![EpisodeWithAction {
                    action: ElementAction::Deleted,
                    episode: existing.clone(),
                }],
            });
            Ok(existing)
        } else {
            Err(SourcesError::UnableToFindEpisodes(
                format!(
                    "library: {} Episode: {}x{}x{}",
                    library_id, serie_id, season, number
                ),
                "remove_episode".to_string(),
            )
            .into())
        }
    }

    pub async fn refresh_episodes(
        &self,
        library_id: &str,
        serie_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Episode>> {
        let ids = self
            .get_serie_ids(library_id, serie_id, requesting_user)
            .await?;
        let existing_episodes = self
            .get_episodes(
                library_id,
                EpisodeQuery {
                    serie_ref: Some(serie_id.to_string()),
                    ..Default::default()
                },
                requesting_user,
            )
            .await?;
        let existing_ids_by_episode: HashMap<(u32, u32), RsIds> = existing_episodes
            .into_iter()
            .map(|episode| ((episode.season, episode.number), RsIds::from(episode)))
            .collect();
        let mut all_episodes = self
            .lookup_episodes_metadata(library_id, serie_id, &ids, requesting_user)
            .await?;
        if all_episodes.is_empty() {
            return Err(SourcesError::NotFound(Some(format!(
                "refresh_episodes - Unable to find episodes for {:?}",
                ids
            )))
            .into());
        }
        for episode in &mut all_episodes {
            if let Some(existing_ids) =
                existing_ids_by_episode.get(&(episode.season, episode.number))
            {
                let mut merged_ids = RsIds::from(episode.clone());
                merged_ids.merge(existing_ids);
                episode.apply_rs_ids(&merged_ids);
            }
        }
        let store = self.store.get_library_store_optional(library_id).ok_or(
            Error::LibraryStoreNotFoundFor(
                library_id.clone().to_string(),
                "refresh_episodes".to_string(),
            ),
        )?;
        store
            .remove_all_serie_episodes(serie_id.to_string())
            .await?;
        let mut new_episodes: Vec<Episode> = vec![];
        for episode in all_episodes {
            let episode = self
                .add_episode(library_id, episode, requesting_user)
                .await?;
            new_episodes.push(episode)
        }
        Ok(new_episodes)
    }

    #[async_recursion]
    pub async fn episode_image(
        &self,
        library_id: &str,
        serie_id: &str,
        season: &u32,
        episode: &u32,
        size: Option<ImageSize>,
        requesting_user: &ConnectedUser,
    ) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        use super::entity_images::EntityImageConfig;

        if RsIds::is_id(serie_id) {
            let serie_ids: RsIds = serie_id.to_string().try_into()?;
            let store = self.store.get_library_store_optional(library_id).ok_or(
                Error::LibraryStoreNotFoundFor(
                    library_id.clone().to_string(),
                    "episode_image".to_string(),
                ),
            )?;
            let existing_serie = store.get_serie_by_external_id(serie_ids.clone()).await?;
            if let Some(existing_serie) = existing_serie {
                return self
                    .episode_image(
                        library_id,
                        &existing_serie.item.id,
                        season,
                        episode,
                        size,
                        requesting_user,
                    )
                    .await;
            }
            let composite_id = format!("{}-episode-{}x{}", serie_id, season, episode);
            let query = RsLookupQuery::Episode(RsLookupEpisode {
                ids: Some(serie_ids),
                season: *season,
                number: Some(*episode),
                ..Default::default()
            });
            let config = EntityImageConfig {
                folder: ".series",
                cache_prefix: "serie",
            };
            self.serve_cached_entity_image(
                library_id,
                &composite_id,
                query,
                &ImageType::Still,
                &config,
                requesting_user,
            )
            .await
        } else {
            let folder = format!(".series/{}", serie_id);
            let entity_id = format!("{}.{}", season, episode);
            let config = EntityImageConfig {
                folder: &folder,
                cache_prefix: "episode",
            };
            self.serve_local_entity_image(
                library_id,
                &entity_id,
                &ImageType::Still,
                size,
                &config,
                requesting_user,
                self.refresh_episode_image(library_id, serie_id, season, episode, requesting_user),
            )
            .await
        }
    }

    pub async fn get_episode_ids(
        &self,
        library_id: &str,
        serie_id: &str,
        season: u32,
        episode: u32,
        requesting_user: &ConnectedUser,
    ) -> RsResult<RsIds> {
        let episode = self
            .get_episode(
                library_id,
                serie_id.to_string(),
                season,
                episode,
                requesting_user,
            )
            .await?;
        let ids: RsIds = episode.into();
        Ok(ids)
    }

    /// download and update image
    pub async fn refresh_episode_image(
        &self,
        library_id: &str,
        serie_id: &str,
        season: &u32,
        episode: &u32,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        let serie_ids: RsIds = self
            .get_serie_ids(library_id, serie_id, requesting_user)
            .await?;
        let query = RsLookupQuery::Episode(RsLookupEpisode {
            ids: Some(serie_ids),
            season: *season,
            number: Some(*episode),
            ..Default::default()
        });
        let reader = self
            .download_entity_image(
                query,
                Some(library_id.to_string()),
                &ImageType::Still,
                requesting_user,
            )
            .await?;
        self.update_episode_image(
            library_id,
            serie_id,
            season,
            episode,
            reader,
            &ConnectedUser::ServerAdmin,
        )
        .await?;
        Ok(())
    }

    pub async fn update_episode_image<T: AsyncRead>(
        &self,
        library_id: &str,
        serie_id: &str,
        season: &u32,
        episode: &u32,
        reader: T,
        requesting_user: &ConnectedUser,
    ) -> Result<()> {
        self.update_library_image(
            library_id,
            &format!(".series/{}", serie_id),
            &format!("{}.{}", season, episode),
            &Some(ImageType::Still),
            &None,
            reader,
            requesting_user,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::should_retry_episode_lookup_with_enriched_ids;
    use rs_plugin_common_interfaces::domain::rs_ids::RsIds;

    #[test]
    fn enriched_episode_lookup_retries_when_new_external_ids_are_added() {
        let original = RsIds::from_tmdb(203744);
        let mut enriched = RsIds::from_tmdb(203744);
        enriched.set("tvdb", 421070u64);

        assert!(should_retry_episode_lookup_with_enriched_ids(
            &original,
            &enriched
        ));
    }

    #[test]
    fn enriched_episode_lookup_does_not_retry_when_external_ids_are_unchanged() {
        let original = RsIds::from_tmdb(203744);
        let enriched = RsIds::from_tmdb(203744);

        assert!(!should_retry_episode_lookup_with_enriched_ids(
            &original,
            &enriched
        ));
    }
}
