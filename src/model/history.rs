use std::collections::{HashMap, HashSet};

use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, MediaType};

use crate::domain::{
    book::Book, episode::Episode, library::LibraryType, movie::Movie, serie::Serie,
};
use crate::tools::log::{log_info, LogServiceType};

use super::{
    books::BookQuery,
    episodes::EpisodeQuery,
    movies::MovieQuery,
    series::SerieQuery,
    store::sql::users::{HistoryIdRewrite, ProgressIdRewrite},
    ModelController,
};

const HISTORY_MIGRATION: &str = "canonical_history_ids_v5";

fn non_empty_id(id: Option<&str>) -> Option<&str> {
    id.map(str::trim).filter(|id| !id.is_empty())
}

fn series_history_key(serie: &Serie) -> String {
    non_empty_id(serie.imdb.as_deref())
        .map(|imdb| format!("imdb/{imdb}"))
        .unwrap_or_else(|| format!("redseat/{}", serie.id))
}

pub fn series_history_id(serie: &Serie) -> String {
    format!("series:{}", series_history_key(serie))
}

pub fn movie_history_id(movie: &Movie) -> String {
    non_empty_id(movie.imdb.as_deref())
        .map(|imdb| format!("movie:imdb/{imdb}"))
        .unwrap_or_else(|| format!("movie:redseat/{}", movie.id))
}

fn book_history_key(book: &Book) -> String {
    let ids: RsIds = book.clone().into();
    for provider in ["isbn13", "oleid", "olwid", "gbvid", "asin"] {
        if let Some(value) = non_empty_id(ids.get(provider)) {
            return format!("{provider}/{value}");
        }
    }
    if let Some((provider, value)) = ids.iter().find(|(provider, value)| {
        !matches!(provider.as_str(), "redseat" | "volume" | "chapter")
            && non_empty_id(Some(value.as_str())).is_some()
    }) {
        return format!("{provider}/{value}");
    }
    format!("redseat/{}", book.id)
}

pub fn book_history_id(book: &Book) -> String {
    format!("book:{}", book_history_key(book))
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

pub fn book_history_ids(book: &Book) -> RsIds {
    let mut ids: RsIds = book.clone().into();
    ids.remove("volume");
    ids.remove("chapter");
    ids_with_history_id(ids, book_history_id(book))
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
        MediaType::Book if id.starts_with("book:") => id,
        MediaType::Book => typed_alias("book", &id).unwrap_or(id),
        _ => id,
    }
}

pub fn direct_history_ids(id: String) -> crate::Result<RsIds> {
    let mut ids = RsIds::try_from(id.clone())?;
    for kind in [MediaType::Movie, MediaType::Episode, MediaType::Book] {
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
        non_empty_id(serie.imdb.as_deref()).map(|id| format!("imdb/{id}")),
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

fn book_history_aliases(book: &Book) -> HashSet<String> {
    let ids = book_history_ids(book);
    ids.as_all_ids()
        .into_iter()
        .flat_map(|id| [Some(id.clone()), typed_alias("book", &id)])
        .flatten()
        .collect()
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
        MediaType::Movie => ["movie:imdb/", "movie:redseat/"]
            .into_iter()
            .any(|prefix| non_empty_id(id.strip_prefix(prefix)).is_some()),
        MediaType::Episode => {
            let Some(path) = id.strip_prefix("episode:imdb/") else {
                return false;
            };
            let mut parts = path.split('/');
            matches!(
                (parts.next(), parts.next(), parts.next(), parts.next()),
                (Some(imdb), Some(season), Some(number), None)
                    if non_empty_id(Some(imdb)).is_some()
                        && season.parse::<u32>().is_ok()
                        && number.parse::<u32>().is_ok()
            )
        }
        MediaType::Book => id
            .strip_prefix("book:")
            .and_then(|value| value.split_once('/'))
            .is_some_and(|(provider, value)| {
                non_empty_id(Some(provider)).is_some() && non_empty_id(Some(value)).is_some()
            }),
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

        let tombstones_removed = self.store.purge_watched_tombstones().await?;
        let mut watched = self.store.get_all_watched().await?;
        let mut progress = self.store.get_all_view_progress_rows().await?;
        watched.sort_by(|a, b| b.modified.cmp(&a.modified));
        progress.sort_by(|a, b| b.modified.cmp(&a.modified));
        let legacy_movie_ids: HashSet<String> = watched
            .iter()
            .map(|row| (&row.kind, &row.id))
            .chain(progress.iter().map(|row| (&row.kind, &row.id)))
            .filter(|(kind, id)| {
                **kind == MediaType::Movie && !is_current_history_id(kind, id)
            })
            .map(|(_, id)| id.clone())
            .collect();
        let legacy_episode_ids: HashSet<String> = watched
            .iter()
            .map(|row| (&row.kind, &row.id))
            .chain(progress.iter().map(|row| (&row.kind, &row.id)))
            .filter(|(kind, id)| {
                **kind == MediaType::Episode && !is_current_history_id(kind, id)
            })
            .map(|(_, id)| id.clone())
            .collect();
        // Every book row is considered so provider aliases can converge on the
        // strongest canonical identifier currently available for the book.
        let book_ids: HashSet<String> = watched
            .iter()
            .filter(|row| row.kind == MediaType::Book)
            .map(|row| row.id.clone())
            .collect();

        if legacy_movie_ids.is_empty() && legacy_episode_ids.is_empty() && book_ids.is_empty() {
            self.store
                .complete_data_migration(HISTORY_MIGRATION)
                .await?;
            return Ok(());
        }

        let libraries = self.store.get_libraries().await?;
        let mut movie_candidates = HashMap::<String, HashSet<String>>::new();
        let mut episode_candidates = HashMap::<String, HashSet<(String, String)>>::new();
        let mut book_candidates = HashMap::<String, HashSet<String>>::new();

        for library in libraries {
            if !matches!(
                library.kind,
                LibraryType::Movies | LibraryType::Shows | LibraryType::Books
            ) {
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

            if library.kind == LibraryType::Books && !book_ids.is_empty() {
                for book in store.get_books(BookQuery::default()).await? {
                    let book = book.item;
                    let canonical_id = book_history_id(&book);
                    for alias in book_history_aliases(&book) {
                        if alias != canonical_id && book_ids.contains(&alias) {
                            add_candidate(&mut book_candidates, alias, canonical_id.clone());
                        }
                    }
                }
            }
        }

        let movie_targets = unique_candidates(movie_candidates);
        let episode_targets = unique_candidates(episode_candidates);
        let book_targets = unique_candidates(book_candidates);
        let watched_rewrites = watched
            .into_iter()
            .filter_map(|watched| {
                let user_ref = watched.user_ref?;
                let new_id = match watched.kind {
                    MediaType::Movie => movie_targets.get(&watched.id)?.clone(),
                    MediaType::Episode => episode_targets.get(&watched.id)?.0.clone(),
                    MediaType::Book => book_targets.get(&watched.id)?.clone(),
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
                "History migration complete: watched rewrites={}, progress rewrites={}, tombstones removed={}",
                watched_count, progress_count, tombstones_removed
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
        old_serie: &Serie,
        new_serie: &Serie,
    ) -> crate::Result<()> {
        let old_prefix = format!("episode:{}/", series_history_key(old_serie));
        let new_prefix = format!("episode:{}/", series_history_key(new_serie));
        if old_prefix == new_prefix {
            return Ok(());
        }

        let watched_rewrites = self
            .store
            .get_all_watched()
            .await?
            .into_iter()
            .filter(|row| row.kind == MediaType::Episode)
            .filter_map(|row| {
                let suffix = row.id.strip_prefix(&old_prefix)?;
                Some(HistoryIdRewrite {
                    new_id: format!("{new_prefix}{suffix}"),
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
                let suffix = row.id.strip_prefix(&old_prefix)?;
                Some(ProgressIdRewrite {
                    new_id: format!("{new_prefix}{suffix}"),
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

#[cfg(test)]
mod tests {
    use super::{book_history_id, book_history_ids, normalize_history_id};
    use crate::domain::book::Book;
    use rs_plugin_common_interfaces::{domain::other_ids::OtherIds, MediaType};

    #[test]
    fn book_history_prefers_isbn13_and_keeps_provider_aliases() {
        let book = Book {
            id: "book-local".to_string(),
            name: "Book".to_string(),
            isbn13: Some("9783161484100".to_string()),
            openlibrary_edition_id: Some("OL123M".to_string()),
            otherids: Some(OtherIds(vec!["goodreads:456".to_string()])),
            volume: Some(2.0),
            ..Default::default()
        };

        assert_eq!(book_history_id(&book), "book:isbn13/9783161484100");
        let ids = book_history_ids(&book).as_all_ids();
        assert!(ids.contains(&"book:isbn13/9783161484100".to_string()));
        assert!(ids.contains(&"oleid:OL123M".to_string()));
        assert!(ids.contains(&"goodreads:456".to_string()));
        assert!(!ids.iter().any(|id| id.starts_with("volume:")));
    }

    #[test]
    fn book_history_falls_back_to_local_id_and_normalizes_direct_ids() {
        let book = Book {
            id: "book-local".to_string(),
            name: "Book".to_string(),
            ..Default::default()
        };
        assert_eq!(book_history_id(&book), "book:redseat/book-local");
        assert_eq!(
            normalize_history_id(&MediaType::Book, "isbn13:9783161484100".to_string()),
            "book:isbn13/9783161484100"
        );
        assert_eq!(
            normalize_history_id(
                &MediaType::Book,
                "book:isbn13/9783161484100".to_string()
            ),
            "book:isbn13/9783161484100"
        );
    }
}
