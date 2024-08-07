use std::{collections::{BTreeMap, HashMap}, io::{self, ErrorKind}, ops::Add, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use futures::TryStreamExt;
use tokio::{fs::File, io::{AsyncReadExt, AsyncWriteExt, BufReader}, sync::{Mutex, RwLock}};
use tokio_util::io::StreamReader;
use crate::{error::RsResult, server::get_server_file_path_array, tools::{get_time, log::{log_info, LogServiceType}}, Error};
use async_compression::tokio::bufread::GzipDecoder;

#[derive(Debug, Clone)]
pub struct ImdbContext {
    ratings: Arc<RwLock<HashMap<String, (f32, u64)>>>,
    freshness: Arc<Mutex<u64>>
}

impl ImdbContext {
    pub fn new() -> Self {
        Self { ratings: Arc::new(RwLock::new(HashMap::new())), freshness: Arc::new(Mutex::new(0)) }
    }
}

impl ImdbContext {
    pub async fn get_sync_rating(&self, imdb: &str) -> Option<(f32, u64)> {
        let ratings =self.ratings.read().await;
           let r = ratings.get(imdb);
           r.map(|r| (r.0, r.1))
    }

    pub async fn get_rating(&self, imdb: &str) -> RsResult<Option<(f32, u64)>> {
        let mut freshness = self.freshness.lock().await;
        let stale = get_time() - Duration::from_secs(86400);

        if freshness.lt(&stale.as_secs()) {
            
            let fresh = self.refresh().await?;
            *freshness = fresh;
            Ok(self.get_sync_rating(imdb).await)
        } else {
            Ok(self.get_sync_rating(imdb).await)
        }
    }

    pub async fn refresh(&self) -> RsResult<u64> {
        let mut map_write = self.ratings.write().await;
        let local_path = get_server_file_path_array(vec!["imdb_cache.tsv"]).await?;
        let now = get_time().as_secs();
        let m = if let Ok(meta) = local_path.metadata() {
            if let Ok(modified) = meta.modified() {
                modified.duration_since(UNIX_EPOCH).unwrap().as_secs()
            } else {
                0
            }
        } else {
            0
        };
        let text = if now - m > 50000 {
            log_info(LogServiceType::Other, "Refreshing IMDB ratings".to_owned());
            map_write.clear();
            let reader = reqwest::get("https://datasets.imdbws.com/title.ratings.tsv.gz").await?.bytes_stream().map_err(|e| io::Error::new(ErrorKind::Other, e));
            let mut decoder = GzipDecoder::new(StreamReader::new(reader));
            let mut text = String::new();
            decoder.read_to_string(&mut text).await?;
            File::create(local_path).await?.write_all(text.as_bytes()).await?;
            text
        } else {
            log_info(LogServiceType::Other, "Loading IMDB ratings in memory".to_owned());
            let mut text = String::new();
            File::open(local_path).await?.read_to_string(&mut text).await?;
            text
        };
        for line in text.lines().skip(1) {
            let separated = line.split("\t").collect::<Vec<_>>();
            if separated.len() == 3 {
                map_write.insert(separated.get(0).unwrap().to_string(), (separated.get(1).unwrap().parse().map_err(|_| Error::GenericRedseatError)?, separated.get(2).unwrap().parse().map_err(|_| Error::GenericRedseatError)?));
            }
        }
        Ok(now)
    }
}