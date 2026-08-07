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

fn series_history_key(serie: &Serie) -> String {
    if let Some(imdb) = &serie.imdb {
        format!("imdb/{imdb}")
    } else if let Some(tmdb) = serie.tmdb {
        format!("tmdb/{tmdb}")
    } else if let Some(tvdb) = serie.tvdb {
        format!("tvdb/{tvdb}")
    } else {
        format!("redseat/{}", serie.id)
    }
}

pub fn series_history_id(serie: &Serie) -> String {
    format!("series:{}", series_history_key(serie))
}

pub fn movie_history_id(movie: &Movie) -> String {
    if let Some(imdb) = &movie.imdb {
        format!("movie:imdb/{imdb}")
    } else if let Some(tmdb) = movie.tmdb {
        format!("movie:tmdb/{tmdb}")
    } else {
        format!("movie:redseat/{}", movie.id)
    }
}

pub fn episode_history_id(serie: &Serie, episode: &Episode) -> String {
    format!(
        "episode:{}/{}/{}",
        series_history_key(serie),
        episode.season,
        episode.number
    )
}

pub fn history_id_rsids(id: String) -> RsIds {
    RsIds::try_from(id).expect("history ids must stay in key:value format")
}

fn dedup_ids(ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(id.clone())).collect()
}

fn movie_legacy_ids(movie: &Movie) -> Vec<String> {
    let ids: Vec<String> = RsIds::from(movie.clone()).into();
    dedup_ids(ids)
}

fn episode_legacy_ids(episode: &Episode) -> Vec<String> {
    let ids: Vec<String> = RsIds::from(episode.clone()).into();
    dedup_ids(ids)
}

impl ModelController {
    pub async fn migrate_history_ids(&self) -> crate::Result<()> {
        let watched = self.store.get_all_watched().await?;
        let progress = self.store.get_all_view_progress_rows().await?;
        let has_history = watched
            .iter()
            .any(|row| matches!(row.kind, MediaType::Movie | MediaType::Episode))
            || progress
                .iter()
                .any(|row| matches!(row.kind, MediaType::Movie | MediaType::Episode));
        if !has_history {
            return Ok(());
        }

        let libraries = self.store.get_libraries().await?;
        let mut movie_targets = HashMap::<String, String>::new();
        let mut episode_targets = HashMap::<String, (String, String)>::new();

        for library in libraries {
            if !matches!(library.kind, LibraryType::Movies | LibraryType::Shows) {
                continue;
            }
            let store = self.store.get_library_store(&library.id)?;

            for movie in store.get_movies(MovieQuery::default()).await? {
                let canonical_id = movie_history_id(&movie);
                for legacy_id in movie_legacy_ids(&movie) {
                    if legacy_id != canonical_id {
                        movie_targets.entry(legacy_id).or_insert_with(|| canonical_id.clone());
                    }
                }
            }

            let series = store.get_series(SerieQuery::default()).await?;
            let series_by_ref: HashMap<String, Serie> = series
                .into_iter()
                .map(|serie| (serie.item.id.clone(), serie.item))
                .collect();
            for episode in store.get_episodes(EpisodeQuery::default()).await? {
                let Some(serie) = series_by_ref.get(&episode.serie) else {
                    continue;
                };
                let canonical_id = episode_history_id(serie, &episode);
                let parent_id = series_history_id(serie);
                for legacy_id in episode_legacy_ids(&episode) {
                    if legacy_id != canonical_id {
                        episode_targets
                            .entry(legacy_id)
                            .or_insert_with(|| (canonical_id.clone(), parent_id.clone()));
                    }
                }
            }
        }

        let watched_rewrites = watched
            .into_iter()
            .filter_map(|watched| match watched.kind {
                MediaType::Movie => movie_targets.get(&watched.id).map(|new_id| HistoryIdRewrite {
                    kind: watched.kind,
                    old_id: watched.id,
                    new_id: new_id.clone(),
                    user_ref: watched.user_ref.unwrap_or_default(),
                }),
                MediaType::Episode => {
                    episode_targets
                        .get(&watched.id)
                        .map(|(new_id, _)| HistoryIdRewrite {
                            kind: watched.kind,
                            old_id: watched.id,
                            new_id: new_id.clone(),
                            user_ref: watched.user_ref.unwrap_or_default(),
                        })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let progress_rewrites = progress
            .into_iter()
            .filter_map(|progress| match progress.kind {
                MediaType::Movie => movie_targets.get(&progress.id).map(|new_id| ProgressIdRewrite {
                    kind: progress.kind,
                    old_id: progress.id,
                    new_id: new_id.clone(),
                    new_parent: None,
                    user_ref: progress.user_ref,
                }),
                MediaType::Episode => {
                    episode_targets
                        .get(&progress.id)
                        .map(|(new_id, parent_id)| ProgressIdRewrite {
                            kind: progress.kind,
                            old_id: progress.id,
                            new_id: new_id.clone(),
                            new_parent: Some(parent_id.clone()),
                            user_ref: progress.user_ref,
                        })
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        let (watched_count, progress_count) = self
            .store
            .apply_history_rewrites(watched_rewrites, progress_rewrites)
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
}
