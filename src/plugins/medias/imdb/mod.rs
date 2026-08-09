use crate::{
    error::RsResult,
    server::get_server_file_path_array,
    tools::{
        get_time,
        log::{log_info, log_warn, LogServiceType},
    },
    Error,
};
use async_compression::tokio::bufread::GzipDecoder;
use futures::TryStreamExt;
use std::{
    collections::{HashMap, HashSet},
    io::{self, ErrorKind},
    sync::Arc,
    time::UNIX_EPOCH,
};
use tokio::{
    fs::{self, File},
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, RwLock},
};
use tokio_util::io::StreamReader;

#[derive(Debug, Clone)]
pub struct ImdbContext {
    ratings: Arc<RwLock<HashMap<String, (f32, u64)>>>,
    rating_cache: Arc<Mutex<ImdbRatingCache>>,
    episode_ids: Arc<RwLock<HashMap<(String, u32, u32), String>>>,
    episode_cache: Arc<Mutex<ImdbEpisodeCache>>,
}

#[derive(Debug, Default)]
struct ImdbRatingCache {
    freshness: u64,
    download_retry_after: u64,
}

#[derive(Debug, Default)]
struct ImdbEpisodeCache {
    source_modified: u64,
    loaded_parents: HashSet<String>,
    download_retry_after: u64,
}

impl ImdbContext {
    pub fn new() -> Self {
        Self {
            ratings: Arc::new(RwLock::new(HashMap::new())),
            rating_cache: Arc::new(Mutex::new(ImdbRatingCache::default())),
            episode_ids: Arc::new(RwLock::new(HashMap::new())),
            episode_cache: Arc::new(Mutex::new(ImdbEpisodeCache::default())),
        }
    }
}

impl ImdbContext {
    pub async fn prime_episode_ids<'a>(
        &self,
        parent_imdb_ids: impl IntoIterator<Item = &'a str>,
    ) -> RsResult<()> {
        let requested: HashSet<String> = parent_imdb_ids
            .into_iter()
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
        if requested.is_empty() {
            return Ok(());
        }

        let mut cache = self.episode_cache.lock().await;
        let local_path = get_server_file_path_array(vec!["imdb_episodes.tsv.gz"]).await?;
        let now = get_time().as_secs();
        let mut source_modified = file_modified_seconds(&local_path).await;

        let source_is_stale = source_modified == 0
            || now.saturating_sub(source_modified) > 86_400;
        if source_is_stale && now >= cache.download_retry_after {
            log_info(
                LogServiceType::Other,
                "Refreshing IMDB episode identifiers".to_owned(),
            );
            match download_episode_dataset(&local_path).await {
                Ok(()) => {
                    cache.download_retry_after = 0;
                    source_modified = file_modified_seconds(&local_path).await;
                }
                Err(error) => {
                    cache.download_retry_after = now.saturating_add(3_600);
                    if source_modified == 0 {
                        return Err(error);
                    }
                    log_warn(
                        LogServiceType::Other,
                        format!(
                            "Unable to refresh IMDB episode identifiers; using stale cache: {:#}",
                            error
                        ),
                    );
                }
            }
        } else if source_modified == 0 {
            return Err(Error::Message(
                "IMDB episode identifier download is temporarily backed off".to_string(),
            ));
        }

        if cache.source_modified != source_modified {
            cache.source_modified = source_modified;
            cache.loaded_parents.clear();
            self.episode_ids.write().await.clear();
        }

        let missing: HashSet<String> = requested
            .difference(&cache.loaded_parents)
            .cloned()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }

        log_info(
            LogServiceType::Other,
            format!(
                "Loading IMDB episode identifiers for {} series",
                missing.len()
            ),
        );
        let file = File::open(local_path).await?;
        let decoder = GzipDecoder::new(BufReader::new(file));
        let mut lines = BufReader::new(decoder).lines();
        let mut found = HashMap::new();
        while let Some(line) = lines.next_line().await? {
            if let Some((parent, season, episode, imdb)) = parse_episode_line(&line, &missing) {
                found.insert((parent, season, episode), imdb);
            }
        }
        self.episode_ids.write().await.extend(found);
        cache.loaded_parents.extend(missing);
        Ok(())
    }

    pub async fn episode_ids(
        &self,
        parent_imdb_id: &str,
    ) -> RsResult<HashMap<(u32, u32), String>> {
        self.prime_episode_ids([parent_imdb_id]).await?;
        let ids = self.episode_ids.read().await;
        Ok(ids
            .iter()
            .filter_map(|((parent, season, episode), imdb)| {
                (parent == parent_imdb_id).then(|| ((*season, *episode), imdb.clone()))
            })
            .collect())
    }

    pub async fn get_sync_rating(&self, imdb: &str) -> Option<(f32, u64)> {
        let ratings = self.ratings.read().await;
        let r = ratings.get(imdb);
        r.map(|r| (r.0, r.1))
    }

    pub async fn get_rating(&self, imdb: &str) -> RsResult<Option<(f32, u64)>> {
        let now = get_time().as_secs();
        let stale_before = now.saturating_sub(86_400);
        let mut cache = self.rating_cache.lock().await;

        if cache.freshness < stale_before && now >= cache.download_retry_after {
            match self.refresh().await {
                Ok(freshness) => {
                    cache.freshness = freshness;
                    cache.download_retry_after = 0;
                }
                Err(error) => {
                    cache.download_retry_after = now.saturating_add(3_600);
                    log_warn(
                        LogServiceType::Other,
                        format!(
                            "Unable to refresh IMDB ratings; using cached values: {:#}",
                            error
                        ),
                    );
                }
            }
        }
        drop(cache);
        Ok(self.get_sync_rating(imdb).await)
    }

    pub async fn refresh(&self) -> RsResult<u64> {
        let local_path = get_server_file_path_array(vec!["imdb_cache.tsv"]).await?;
        let now = get_time().as_secs();
        let modified = file_modified_seconds(&local_path).await;
        let text = if now.saturating_sub(modified) > 50_000 {
            if self.ratings.read().await.is_empty() && modified > 0 {
                if let Ok(stale_text) = read_text_file(&local_path).await {
                    if let Ok(stale_ratings) = parse_ratings(&stale_text) {
                        *self.ratings.write().await = stale_ratings;
                    }
                }
            }
            log_info(LogServiceType::Other, "Refreshing IMDB ratings".to_owned());
            let reader = reqwest::get("https://datasets.imdbws.com/title.ratings.tsv.gz")
                .await?
                .error_for_status()?
                .bytes_stream()
                .map_err(|e| io::Error::new(ErrorKind::Other, e));
            let mut decoder = GzipDecoder::new(StreamReader::new(reader));
            let mut text = String::new();
            decoder.read_to_string(&mut text).await?;
            File::create(local_path)
                .await?
                .write_all(text.as_bytes())
                .await?;
            text
        } else {
            log_info(
                LogServiceType::Other,
                "Loading IMDB ratings in memory".to_owned(),
            );
            read_text_file(&local_path).await?
        };
        *self.ratings.write().await = parse_ratings(&text)?;
        Ok(now)
    }
}

async fn read_text_file(path: &std::path::Path) -> RsResult<String> {
    let mut text = String::new();
    File::open(path).await?.read_to_string(&mut text).await?;
    Ok(text)
}

fn parse_ratings(text: &str) -> RsResult<HashMap<String, (f32, u64)>> {
    let mut ratings = HashMap::new();
    for line in text.lines().skip(1) {
        let mut columns = line.split('\t');
        let Some(imdb) = columns.next() else {
            continue;
        };
        let Some(rating) = columns.next() else {
            continue;
        };
        let Some(votes) = columns.next() else {
            continue;
        };
        if columns.next().is_some() {
            continue;
        }
        ratings.insert(
            imdb.to_string(),
            (
                rating.parse().map_err(|_| Error::GenericRedseatError)?,
                votes.parse().map_err(|_| Error::GenericRedseatError)?,
            ),
        );
    }
    Ok(ratings)
}

async fn download_episode_dataset(local_path: &std::path::Path) -> RsResult<()> {
    let download_path = local_path.with_extension("tsv.gz.download");
    let reader = reqwest::get("https://datasets.imdbws.com/title.episode.tsv.gz")
        .await?
        .error_for_status()?
        .bytes_stream()
        .map_err(|e| io::Error::new(ErrorKind::Other, e));
    let mut stream = StreamReader::new(reader);
    let mut file = File::create(&download_path).await?;
    tokio::io::copy(&mut stream, &mut file).await?;
    file.flush().await?;
    drop(file);
    if fs::try_exists(local_path).await? {
        fs::remove_file(local_path).await?;
    }
    fs::rename(download_path, local_path).await?;
    Ok(())
}

async fn file_modified_seconds(path: &std::path::Path) -> u64 {
    match fs::metadata(path).await.and_then(|metadata| metadata.modified()) {
        Ok(modified) => modified
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
        Err(_) => 0,
    }
}

fn parse_episode_line(
    line: &str,
    requested_parents: &HashSet<String>,
) -> Option<(String, u32, u32, String)> {
    let mut columns = line.split('\t');
    let imdb = columns.next()?;
    let parent = columns.next()?;
    if !requested_parents.contains(parent) {
        return None;
    }
    let season = columns.next()?.parse().ok()?;
    let episode = columns.next()?.parse().ok()?;
    Some((
        parent.to_owned(),
        season,
        episode,
        imdb.to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{parse_episode_line, parse_ratings, ImdbContext};
    use crate::tools::get_time;
    use std::collections::HashSet;

    #[test]
    fn parses_sugar_episode_mapping() {
        let requested = HashSet::from(["tt16418808".to_string()]);
        assert_eq!(
            parse_episode_line("tt20918504\ttt16418808\t1\t1", &requested),
            Some((
                "tt16418808".to_string(),
                1,
                1,
                "tt20918504".to_string()
            ))
        );
    }

    #[test]
    fn ignores_other_series_and_unknown_episode_numbers() {
        let requested = HashSet::from(["tt16418808".to_string()]);
        assert!(parse_episode_line("tt00000001\ttt00000002\t1\t1", &requested).is_none());
        assert!(parse_episode_line("tt20918504\ttt16418808\t\\N\t1", &requested).is_none());
    }

    #[test]
    fn parses_imdb_rating_rows() {
        let ratings = parse_ratings(
            "tconst\taverageRating\tnumVotes\ntt20918504\t7.4\t1200\n",
        )
        .unwrap();
        assert_eq!(ratings.get("tt20918504"), Some(&(7.4, 1200)));
    }

    #[tokio::test]
    async fn rating_download_backoff_returns_cached_values_without_retrying() {
        let context = ImdbContext::new();
        context
            .ratings
            .write()
            .await
            .insert("tt20918504".to_string(), (7.4, 1200));
        context.rating_cache.lock().await.download_retry_after =
            get_time().as_secs().saturating_add(3_600);

        assert_eq!(
            context.get_rating("tt20918504").await.unwrap(),
            Some((7.4, 1200))
        );
    }
}
