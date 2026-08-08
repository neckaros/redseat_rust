use std::collections::{HashMap, HashSet};

use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, MediaType};

use crate::domain::{episode::Episode, library::LibraryType, movie::Movie, serie::Serie};
use crate::tools::log::{log_info, LogServiceType};

use super::{
    episodes::EpisodeQuery,
    movies::MovieQuery,
    series::SerieQuery,
    store::sql::users::{HistoryIdRewrite, ProgressIdRewrite},
    ModelController,
};

const HISTORY_MIGRATION: &str = "canonical_history_ids_v3";
const PREVIOUS_HISTORY_MIGRATION: &str = "canonical_history_ids_v2";

fn series_history_key(serie: &Serie) -> String {
    serie
        .imdb
        .as_ref()
        .map(|imdb| format!("imdb/{imdb}"))
        .unwrap_or_else(|| format!("redseat/{}", serie.id))
}

pub fn series_history_id(serie: &Serie) -> String {
    format!("series:{}", series_history_key(serie))
}

pub fn movie_history_id(movie: &Movie) -> String {
    movie
        .imdb
        .as_ref()
        .map(|imdb| format!("movie:imdb/{imdb}"))
        .unwrap_or_else(|| format!("movie:redseat/{}", movie.id))
}

pub fn episode_history_id(serie: &Serie, episode: &Episode) -> String {
    format!(
        "episode:{}/{}/{}",
        series_history_key(serie),
        episode.season,
        episode.number
    )
}

fn ids_with_history_id(mut ids: RsIds, history_id: String) -> RsIds {
    ids.try_add(history_id)
        .expect("history ids must stay in key:value format");
    ids
}

pub fn movie_history_ids(movie: &Movie) -> RsIds {
    ids_with_history_id(movie.clone().into(), movie_history_id(movie))
}

pub fn episode_history_ids(serie: &Serie, episode: &Episode) -> RsIds {
    ids_with_history_id(episode.clone().into(), episode_history_id(serie, episode))
}

pub fn normalize_history_id(kind: &MediaType, id: String) -> String {
    match kind {
        MediaType::Movie if id.starts_with("movie:") => id,
        MediaType::Movie => id
            .strip_prefix("imdb:")
            .map(|value| format!("movie:imdb/{value}"))
            .or_else(|| {
                id.strip_prefix("redseat:")
                    .map(|value| format!("movie:redseat/{value}"))
            })
            .unwrap_or(id),
        MediaType::Episode if id.starts_with("episode:") => id,
        MediaType::Episode => id
            .strip_prefix("redseat:")
            .and_then(normalize_legacy_episode_id)
            .unwrap_or(id),
        _ => id,
    }
}

pub fn direct_history_ids(id: String) -> crate::Result<RsIds> {
    let mut ids = RsIds::try_from(id.clone())?;
    for kind in [MediaType::Movie, MediaType::Episode] {
        let normalized = normalize_history_id(&kind, id.clone());
        if normalized != id {
            ids.try_add(normalized)?;
        }
    }
    Ok(ids)
}

fn normalize_legacy_episode_id(value: &str) -> Option<String> {
    let mut parts = value.rsplitn(3, 'x');
    let number = parts.next()?.parse::<u32>().ok()?;
    let season = parts.next()?.parse::<u32>().ok()?;
    let serie = parts.next()?;
    Some(format!("episode:redseat/{serie}/{season}/{number}"))
}

fn typed_alias(kind: &str, id: &str) -> Option<String> {
    let (provider, value) = id.split_once(':')?;
    Some(format!("{kind}:{provider}/{value}"))
}

fn movie_legacy_ids(movie: &Movie) -> HashSet<String> {
    let ids: RsIds = movie.clone().into();
    ids.as_all_ids()
        .into_iter()
        .flat_map(|id| [Some(id.clone()), typed_alias("movie", &id)])
        .flatten()
        .collect()
}

fn episode_legacy_ids(serie: &Serie, episode: &Episode) -> HashSet<String> {
    let ids: RsIds = episode.clone().into();
    let mut aliases: HashSet<String> = ids
        .as_all_ids()
        .into_iter()
        .flat_map(|id| [Some(id.clone()), typed_alias("episode", &id)])
        .flatten()
        .collect();

    for key in [
        serie.imdb.as_ref().map(|id| format!("imdb/{id}")),
        serie.tmdb.map(|id| format!("tmdb/{id}")),
        serie.tvdb.map(|id| format!("tvdb/{id}")),
        Some(format!("redseat/{}", serie.id)),
    ]
    .into_iter()
    .flatten()
    {
        aliases.insert(format!(
            "episode:{key}/{}/{}",
            episode.season, episode.number
        ));
    }
    aliases
}

fn add_candidate<T: Clone + Eq + std::hash::Hash>(
    candidates: &mut HashMap<String, HashSet<T>>,
    old_id: String,
    target: T,
) {
    candidates.entry(old_id).or_default().insert(target);
}

fn unique_candidates<T>(candidates: HashMap<String, HashSet<T>>) -> HashMap<String, T> {
    candidates
        .into_iter()
        .filter_map(|(old_id, targets)| {
            if targets.len() == 1 {
                targets.into_iter().next().map(|target| (old_id, target))
            } else {
                None
            }
        })
        .collect()
}

fn is_current_history_id(kind: &MediaType, id: &str) -> bool {
    match kind {
        MediaType::Movie => id.starts_with("movie:imdb/") || id.starts_with("movie:redseat/"),
        MediaType::Episode => id.starts_with("episode:imdb/"),
        _ => true,
    }
}

impl ModelController {
    pub async fn migrate_history_ids(&self) -> crate::Result<()> {
        if self
            .store
            .is_data_migration_complete(HISTORY_MIGRATION)
            .await?
        {
            return Ok(());
        }

        let watched = self.store.get_all_watched().await?;
        let progress = self.store.get_all_view_progress_rows().await?;
        let previous_migration_complete = self
            .store
            .is_data_migration_complete(PREVIOUS_HISTORY_MIGRATION)
            .await?;
        let legacy_movie_ids: HashSet<String> = watched
            .iter()
            .map(|row| (&row.kind, &row.id))
            .chain(progress.iter().map(|row| (&row.kind, &row.id)))
            .filter(|(kind, id)| {
                !previous_migration_complete
                    && **kind == MediaType::Movie
                    && !is_current_history_id(kind, id)
            })
            .map(|(_, id)| id.clone())
            .collect();
        let legacy_episode_ids: HashSet<String> = watched
            .iter()
            .map(|row| (&row.kind, &row.id))
            .chain(progress.iter().map(|row| (&row.kind, &row.id)))
            .filter(|(kind, id)| {
                **kind == MediaType::Episode
                    && if previous_migration_complete {
                        id.starts_with("episode:redseat/")
                    } else {
                        !is_current_history_id(kind, id)
                    }
            })
            .map(|(_, id)| id.clone())
            .collect();

        if legacy_movie_ids.is_empty() && legacy_episode_ids.is_empty() {
            self.store
                .complete_data_migration(HISTORY_MIGRATION)
                .await?;
            return Ok(());
        }

        let libraries = self.store.get_libraries().await?;
        let mut movie_candidates = HashMap::<String, HashSet<String>>::new();
        let mut episode_candidates = HashMap::<String, HashSet<(String, String)>>::new();

        for library in libraries {
            if !matches!(library.kind, LibraryType::Movies | LibraryType::Shows) {
                continue;
            }
            let store = self.store.get_library_store(&library.id)?;

            if library.kind == LibraryType::Movies && !legacy_movie_ids.is_empty() {
                for movie in store.get_movies(MovieQuery::default()).await? {
                    let canonical_id = movie_history_id(&movie);
                    for legacy_id in movie_legacy_ids(&movie) {
                        if legacy_id != canonical_id && legacy_movie_ids.contains(&legacy_id) {
                            add_candidate(&mut movie_candidates, legacy_id, canonical_id.clone());
                        }
                    }
                }
            }

            if library.kind == LibraryType::Shows && !legacy_episode_ids.is_empty() {
                let series_by_ref: HashMap<String, Serie> = store
                    .get_series(SerieQuery::default())
                    .await?
                    .into_iter()
                    .map(|serie| (serie.item.id.clone(), serie.item))
                    .collect();
                for episode in store.get_episodes(EpisodeQuery::default()).await? {
                    let Some(serie) = series_by_ref.get(&episode.serie) else {
                        continue;
                    };
                    let target = (
                        episode_history_id(serie, &episode),
                        series_history_id(serie),
                    );
                    for legacy_id in episode_legacy_ids(serie, &episode) {
                        if legacy_episode_ids.contains(&legacy_id) {
                            add_candidate(&mut episode_candidates, legacy_id, target.clone());
                        }
                    }
                }
            }
        }

        let movie_targets = unique_candidates(movie_candidates);
        let episode_targets = unique_candidates(episode_candidates);
        let watched_rewrites = watched
            .into_iter()
            .filter_map(|watched| {
                let user_ref = watched.user_ref?;
                let new_id = match watched.kind {
                    MediaType::Movie => movie_targets.get(&watched.id)?.clone(),
                    MediaType::Episode => episode_targets.get(&watched.id)?.0.clone(),
                    _ => return None,
                };
                Some(HistoryIdRewrite {
                    kind: watched.kind,
                    old_id: watched.id,
                    new_id,
                    user_ref,
                })
            })
            .collect();
        let progress_rewrites = progress
            .into_iter()
            .filter_map(|progress| {
                let (new_id, new_parent) = match progress.kind {
                    MediaType::Movie => (movie_targets.get(&progress.id)?.clone(), None),
                    MediaType::Episode => {
                        let (id, parent) = episode_targets.get(&progress.id)?;
                        (id.clone(), Some(parent.clone()))
                    }
                    _ => return None,
                };
                Some(ProgressIdRewrite {
                    kind: progress.kind,
                    old_id: progress.id,
                    new_id,
                    new_parent,
                    user_ref: progress.user_ref,
                })
            })
            .collect();

        let (watched_count, progress_count) = self
            .store
            .apply_history_rewrites(watched_rewrites, progress_rewrites)
            .await?;
        self.store
            .complete_data_migration(HISTORY_MIGRATION)
            .await?;
        log_info(
            LogServiceType::Database,
            format!(
                "History migration complete: watched rewrites={}, progress rewrites={}",
                watched_count, progress_count
            ),
        );
        Ok(())
    }

    pub async fn migrate_movie_history_id(
        &self,
        old_id: String,
        new_id: String,
    ) -> crate::Result<()> {
        if old_id == new_id {
            return Ok(());
        }
        let watched_rewrites = self
            .store
            .get_all_watched()
            .await?
            .into_iter()
            .filter(|row| row.kind == MediaType::Movie && row.id == old_id)
            .filter_map(|row| {
                Some(HistoryIdRewrite {
                    kind: row.kind,
                    old_id: row.id,
                    new_id: new_id.clone(),
                    user_ref: row.user_ref?,
                })
            })
            .collect();
        let progress_rewrites = self
            .store
            .get_all_view_progress_rows()
            .await?
            .into_iter()
            .filter(|row| row.kind == MediaType::Movie && row.id == old_id)
            .map(|row| ProgressIdRewrite {
                kind: row.kind,
                old_id: row.id,
                new_id: new_id.clone(),
                new_parent: None,
                user_ref: row.user_ref,
            })
            .collect();
        self.store
            .apply_history_rewrites(watched_rewrites, progress_rewrites)
            .await?;
        Ok(())
    }

    pub async fn migrate_series_history_ids(
        &self,
        library_id: &str,
        old_serie: &Serie,
        new_serie: &Serie,
    ) -> crate::Result<()> {
        if series_history_id(old_serie) == series_history_id(new_serie) {
            return Ok(());
        }

        let store = self.store.get_library_store(library_id)?;
        let episode_targets: HashMap<String, String> = store
            .get_episodes(EpisodeQuery {
                serie_ref: Some(new_serie.id.clone()),
                ..Default::default()
            })
            .await?
            .into_iter()
            .map(|episode| {
                (
                    episode_history_id(old_serie, &episode),
                    episode_history_id(new_serie, &episode),
                )
            })
            .collect();
        let watched_rewrites = self
            .store
            .get_all_watched()
            .await?
            .into_iter()
            .filter(|row| row.kind == MediaType::Episode)
            .filter_map(|row| {
                Some(HistoryIdRewrite {
                    new_id: episode_targets.get(&row.id)?.clone(),
                    kind: row.kind,
                    old_id: row.id,
                    user_ref: row.user_ref?,
                })
            })
            .collect();
        let progress_rewrites = self
            .store
            .get_all_view_progress_rows()
            .await?
            .into_iter()
            .filter(|row| row.kind == MediaType::Episode)
            .filter_map(|row| {
                Some(ProgressIdRewrite {
                    new_id: episode_targets.get(&row.id)?.clone(),
                    new_parent: Some(series_history_id(new_serie)),
                    kind: row.kind,
                    old_id: row.id,
                    user_ref: row.user_ref,
                })
            })
            .collect();
        self.store
            .apply_history_rewrites(watched_rewrites, progress_rewrites)
            .await?;
        Ok(())
    }
}
