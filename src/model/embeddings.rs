use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use rs_plugin_common_interfaces::domain::ItemWithRelations;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OwnedRwLockReadGuard, RwLock};

use super::{
    error::Error as ModelError, store::sql::library::embeddings::MediaEmbeddingCounts,
    users::ConnectedUser, ModelController,
};
use crate::{
    domain::{
        library::LibraryRole,
        media::{FileType, Media},
    },
    error::{RsError, RsResult},
    plugins::sources::error::SourcesError,
    server::get_server_local_path,
    tools::{
        embedding::{self, index::VectorIndex, Embedder, EmbedderSpec},
        get_time,
        image_tools::convert_image_reader,
        log::{log_error, log_info, LogServiceType},
        scheduler::{embedding::EmbeddingTask, RsSchedulerWhen, RsTaskType},
        video_tools::{self, VideoTime},
    },
};

/// Frames averaged into the single vector of a video.
const VIDEO_FRAMES: usize = 8;
/// Failed attempts after which a media is no longer offered for embedding.
pub const EMBEDDING_MAX_ATTEMPTS: u32 = 3;
const SEARCH_DEFAULT_LIMIT: usize = 50;
const SEARCH_MAX_LIMIT: usize = 500;
/// Ranked ids resolved to medias per query.
const SEARCH_RESOLVE_CHUNK: usize = 200;

type IndexSlot = Arc<RwLock<Option<VectorIndex>>>;

/// Shared state of the embedding feature: the loaded model and one search index per library.
#[derive(Default)]
pub struct EmbeddingService {
    /// Held while the model is downloaded and loaded, so that only happens once.
    embedder: Mutex<Option<Arc<dyn Embedder>>>,
    downloading: AtomicBool,
    backfill_running: AtomicBool,
    /// `None` until the first search on the library loads its vectors.
    indexes: Mutex<HashMap<String, IndexSlot>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum EmbeddingModelState {
    /// In memory, ready to embed.
    Loaded,
    /// On disk, loaded on first use.
    Available,
    Downloading,
    /// Downloaded on first use.
    Missing,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MediaEmbeddingStatus {
    pub embedder: String,
    pub dimension: usize,
    pub search_dimension: usize,
    /// Library setting `embeddings`: new and existing medias are embedded in the background.
    pub enabled: bool,
    pub model: EmbeddingModelState,
    /// A backfill is running on this server (any library).
    pub running: bool,
    #[serde(flatten)]
    pub counts: MediaEmbeddingCounts,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MediaEmbeddingSearchStatus {
    Ok,
    /// Nothing is embedded yet in this library for the active embedder.
    NoEmbeddings,
    /// The model is being downloaded; text search will work once it is done.
    ModelDownloading,
    /// The reference media of a similarity search has no vector yet.
    MediaNotEmbedded,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MediaEmbeddingHit {
    pub media: ItemWithRelations<Media>,
    /// Cosine similarity with the query, higher is closer.
    pub score: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MediaEmbeddingSearch {
    pub status: MediaEmbeddingSearchStatus,
    pub embedder: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub results: Vec<MediaEmbeddingHit>,
}

impl MediaEmbeddingSearch {
    fn empty(status: MediaEmbeddingSearchStatus, message: &str) -> Self {
        Self {
            status,
            embedder: embedding::active_spec().id.to_string(),
            message: Some(message.to_string()),
            results: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaEmbeddingSearchQuery {
    /// Free text, only used by the text search.
    pub text: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

async fn embedding_models_path() -> RsResult<PathBuf> {
    Ok(get_server_local_path().await?.join("models"))
}

impl ModelController {
    pub fn embedder_spec(&self) -> EmbedderSpec {
        embedding::active_spec()
    }

    /// Returns the active embedder, downloading and loading it on first use.
    pub async fn get_embedder(&self) -> RsResult<Arc<dyn Embedder>> {
        let mut guard = self.embeddings.embedder.lock().await;
        if let Some(embedder) = &*guard {
            return Ok(embedder.clone());
        }
        let models_path = embedding_models_path().await?;
        let must_download = !embedding::active_files_present(&models_path);
        self.embeddings
            .downloading
            .store(must_download, Ordering::Relaxed);
        let loaded = embedding::load_active(&models_path, &|message| {
            log_info(LogServiceType::Source, message)
        })
        .await;
        self.embeddings.downloading.store(false, Ordering::Relaxed);
        let embedder = loaded?;
        log_info(
            LogServiceType::Source,
            format!("Embedding model loaded: {}", embedder.id()),
        );
        *guard = Some(embedder.clone());
        Ok(embedder)
    }

    /// The embedder if it can be had without a long wait. When its files are missing
    /// the download is started in the background and `None` is returned.
    async fn get_embedder_for_search(&self) -> RsResult<Option<Arc<dyn Embedder>>> {
        if let Ok(guard) = self.embeddings.embedder.try_lock() {
            if let Some(embedder) = &*guard {
                return Ok(Some(embedder.clone()));
            }
        }
        if embedding::active_files_present(&embedding_models_path().await?) {
            return Ok(Some(self.get_embedder().await?));
        }
        if !self.embeddings.downloading.load(Ordering::Relaxed) {
            let mc = self.clone();
            tokio::spawn(async move {
                if let Err(error) = mc.get_embedder().await {
                    log_error(
                        LogServiceType::Source,
                        format!("Unable to load embedding model: {:#}", error),
                    );
                }
            });
        }
        Ok(None)
    }

    async fn embedding_model_state(&self) -> RsResult<EmbeddingModelState> {
        if let Ok(guard) = self.embeddings.embedder.try_lock() {
            if guard.is_some() {
                return Ok(EmbeddingModelState::Loaded);
            }
        }
        Ok(
            if embedding::active_files_present(&embedding_models_path().await?) {
                EmbeddingModelState::Available
            } else if self.embeddings.downloading.load(Ordering::Relaxed) {
                EmbeddingModelState::Downloading
            } else {
                EmbeddingModelState::Missing
            },
        )
    }

    /// Whether background embedding is turned on for the library (off by default).
    pub async fn library_embeddings_enabled(&self, library_id: &str) -> bool {
        self.cache_get_library(library_id)
            .await
            .and_then(|library| library.settings.embeddings)
            .unwrap_or(false)
    }

    pub async fn get_media_embeddings_status(
        &self,
        library_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<MediaEmbeddingStatus> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let spec = self.embedder_spec();
        let store = self.store.get_library_store(library_id)?;
        let counts = store
            .count_media_embeddings(spec.id.to_string(), EMBEDDING_MAX_ATTEMPTS)
            .await?;
        Ok(MediaEmbeddingStatus {
            embedder: spec.id.to_string(),
            dimension: spec.dimension,
            search_dimension: spec.search_dimension,
            enabled: self.library_embeddings_enabled(library_id).await,
            model: self.embedding_model_state().await?,
            running: self.embeddings.backfill_running.load(Ordering::Relaxed),
            counts,
        })
    }

    pub async fn count_medias_pending_embedding(&self, library_id: &str) -> RsResult<u64> {
        let store = self.store.get_library_store(library_id)?;
        let counts = store
            .count_media_embeddings(self.embedder_spec().id.to_string(), EMBEDDING_MAX_ATTEMPTS)
            .await?;
        Ok(counts.pending)
    }

    /// Next medias to embed. `pass_started` (ms) keeps medias that already failed
    /// during the current pass out of it.
    pub async fn get_medias_pending_embedding(
        &self,
        library_id: &str,
        pass_started: i64,
        limit: usize,
    ) -> RsResult<Vec<String>> {
        let store = self.store.get_library_store(library_id)?;
        let media_ids = store
            .get_medias_pending_embedding(
                self.embedder_spec().id.to_string(),
                EMBEDDING_MAX_ATTEMPTS,
                pass_started,
                limit,
            )
            .await?;
        Ok(media_ids)
    }

    /// Marks a backfill as running. Returns false when one already is.
    pub fn try_start_embedding_backfill(&self) -> bool {
        !self
            .embeddings
            .backfill_running
            .swap(true, Ordering::SeqCst)
    }

    pub async fn finish_embedding_backfill(&self) {
        // The vision encoder is only worth its memory while a batch is running.
        if let Ok(guard) = self.embeddings.embedder.try_lock() {
            if let Some(embedder) = &*guard {
                embedder.release_image_resources();
            }
        }
        self.embeddings
            .backfill_running
            .store(false, Ordering::SeqCst);
    }

    /// Queues an immediate embedding pass on one library. The library still has to
    /// have the `embeddings` setting on. `retry_failed` forgets recorded failures first.
    pub async fn start_media_embedding_task(
        &self,
        library_id: &str,
        retry_failed: bool,
        requesting_user: &ConnectedUser,
    ) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        self.cache_check_library_notcrypt(library_id).await?;
        if !self.library_embeddings_enabled(library_id).await {
            return Err(RsError::InvalidParams(
                "Embeddings are not enabled for this library".to_string(),
            ));
        }
        if retry_failed {
            let store = self.store.get_library_store(library_id)?;
            store
                .clear_media_embedding_errors(self.embedder_spec().id.to_string())
                .await?;
        }
        self.scheduler
            .add(
                RsTaskType::Embedding,
                RsSchedulerWhen::At(0),
                EmbeddingTask {
                    specific_library: Some(library_id.to_string()),
                },
            )
            .await
    }

    /// Computes and stores the vector of one photo or video. A failure is recorded
    /// against the media so it is not retried forever.
    pub async fn embed_media(&self, library_id: &str, media_id: &str) -> RsResult<()> {
        // Same limit as the rest of media processing: the server cannot read these files.
        self.cache_check_library_notcrypt(library_id).await?;
        let embedder = self.get_embedder().await?;
        let spec = *embedder.spec();
        let store = self.store.get_library_store(library_id)?;

        match self
            .compute_media_embedding(library_id, media_id, &embedder)
            .await
        {
            Ok(vector) => {
                let quantized = embedding::quantize(&vector, spec.quantization_scale);
                store
                    .add_media_embedding(
                        media_id.to_string(),
                        spec.id.to_string(),
                        embedding::to_bytes(&quantized),
                    )
                    .await?;
                // Taken after the row is written: a load that started earlier holds
                // the lock until done, one that starts later reads the row itself.
                let slot = self.embedding_index_slot(library_id).await;
                if let Some(index) = &mut *slot.write().await {
                    index.upsert(media_id, &quantized);
                }
                Ok(())
            }
            Err(error) => {
                // The library is briefly unreadable during an encryption migration:
                // that says nothing about the media.
                if !matches!(
                    error,
                    RsError::Model(ModelError::LibraryEncryptionUnavailable(_))
                ) {
                    store
                        .mark_media_embedding_error(
                            media_id.to_string(),
                            spec.id.to_string(),
                            format!("{:#}", error),
                        )
                        .await?;
                }
                Err(error)
            }
        }
    }

    async fn compute_media_embedding(
        &self,
        library_id: &str,
        media_id: &str,
        embedder: &Arc<dyn Embedder>,
    ) -> RsResult<Vec<f32>> {
        let user = &ConnectedUser::ServerAdmin;
        let store = self.store.get_library_store(library_id)?;
        let source =
            store
                .get_media_source(media_id)
                .await?
                .ok_or(SourcesError::UnableToFindMedia(
                    library_id.to_string(),
                    media_id.to_string(),
                    "embed_media".to_string(),
                ))?;

        let mut images = if source.kind == FileType::Video {
            match self
                .video_embedding_frames(library_id, media_id, user)
                .await
            {
                Ok(frames) => frames,
                Err(error) => {
                    log_error(
                        LogServiceType::Source,
                        format!(
                            "Unable to sample frames of {} for embedding, using its thumbnail: {:#}",
                            media_id, error
                        ),
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if images.is_empty() {
            let thumbnail = self.media_image(library_id, media_id, None, user).await?;
            images.push(
                convert_image_reader(thumbnail.stream, image::ImageFormat::Png, None, true).await?,
            );
        }

        let embedder = embedder.clone();
        let vectors = tokio::task::spawn_blocking(move || {
            images
                .iter()
                .map(|image| embedder.embed_image(image))
                .collect::<Result<Vec<_>, _>>()
        })
        .await
        .map_err(|error| RsError::Error(format!("Join error: {}", error)))??;
        embedding::average_normalized(&vectors)
            .ok_or_else(|| RsError::Error(format!("No embedding computed for {}", media_id)))
    }

    /// A few frames spread evenly over the video. Frames that cannot be extracted are skipped.
    async fn video_embedding_frames(
        &self,
        library_id: &str,
        media_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Vec<Vec<u8>>> {
        let uri = self.get_media_uri(library_id, media_id, Some(240)).await?;
        let duration = video_tools::get_duration(&uri)
            .await?
            .ok_or_else(|| RsError::Error("Unable to get video duration".to_string()))?;

        let mut frames = Vec::with_capacity(VIDEO_FRAMES);
        for index in 0..VIDEO_FRAMES {
            let seconds = duration * (index as f64 + 0.5) / VIDEO_FRAMES as f64;
            match self
                .get_video_thumb(
                    library_id,
                    media_id,
                    VideoTime::Seconds(seconds),
                    image::ImageFormat::Png,
                    None,
                    requesting_user,
                )
                .await
            {
                Ok(frame) => frames.push(frame),
                Err(error) => log_error(
                    LogServiceType::Source,
                    format!(
                        "Unable to extract frame at {}s of {} for embedding: {:#}",
                        seconds, media_id, error
                    ),
                ),
            }
        }
        Ok(frames)
    }

    async fn embedding_index_slot(&self, library_id: &str) -> IndexSlot {
        let mut indexes = self.embeddings.indexes.lock().await;
        indexes.entry(library_id.to_string()).or_default().clone()
    }

    /// The library's index, loaded from the store on first use.
    async fn embedding_index(
        &self,
        library_id: &str,
    ) -> RsResult<OwnedRwLockReadGuard<Option<VectorIndex>>> {
        let slot = self.embedding_index_slot(library_id).await;
        {
            let index = slot.clone().read_owned().await;
            if index.is_some() {
                return Ok(index);
            }
        }
        let mut index = slot.write_owned().await;
        if index.is_none() {
            let spec = self.embedder_spec();
            let store = self.store.get_library_store(library_id)?;
            let (ids, data) = store
                .get_media_embedding_prefixes(spec.id.to_string(), spec.search_dimension)
                .await?;
            let loaded = tokio::task::spawn_blocking(move || {
                let mut loaded = VectorIndex::with_capacity(spec.search_dimension, ids.len());
                let rows = data.chunks_exact(spec.search_dimension);
                for (id, row) in ids.iter().zip(rows) {
                    loaded.upsert(id, bytemuck::cast_slice(row));
                }
                loaded
            })
            .await
            .map_err(|error| RsError::Error(format!("Join error: {}", error)))?;
            *index = Some(loaded);
        }
        Ok(index.downgrade())
    }

    /// Forgets a media in the loaded index. Its rows go with the media in the database.
    pub async fn remove_media_from_embedding_index(&self, library_id: &str, media_id: &str) {
        let slot = {
            let indexes = self.embeddings.indexes.lock().await;
            indexes.get(library_id).cloned()
        };
        if let Some(slot) = slot {
            if let Some(index) = &mut *slot.write().await {
                index.remove(media_id);
            }
        }
    }

    /// Frees the index of a library that is going away.
    pub async fn drop_embedding_index(&self, library_id: &str) {
        self.embeddings.indexes.lock().await.remove(library_id);
    }

    /// Medias whose content is closest to a text query, best first.
    pub async fn search_medias_by_text(
        &self,
        library_id: &str,
        query: MediaEmbeddingSearchQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<MediaEmbeddingSearch> {
        // Checked before anything else so nothing is computed for a user without access.
        self.media_listing_scope(library_id, Default::default(), requesting_user)
            .await?;
        let text = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| RsError::InvalidParams("text is required".to_string()))?
            .to_string();

        if self
            .embedding_index(library_id)
            .await?
            .as_ref()
            .is_none_or(VectorIndex::is_empty)
        {
            return Ok(MediaEmbeddingSearch::empty(
                MediaEmbeddingSearchStatus::NoEmbeddings,
                "No media of this library has been embedded yet",
            ));
        }
        let Some(embedder) = self.get_embedder_for_search().await? else {
            return Ok(MediaEmbeddingSearch::empty(
                MediaEmbeddingSearchStatus::ModelDownloading,
                "The embedding model is being downloaded, try again later",
            ));
        };

        let scale = embedder.spec().quantization_scale;
        let vector = tokio::task::spawn_blocking(move || embedder.embed_text(&text))
            .await
            .map_err(|error| RsError::Error(format!("Join error: {}", error)))??;
        self.search_medias_by_vector(
            library_id,
            embedding::quantize(&vector, scale),
            None,
            query,
            requesting_user,
        )
        .await
    }

    /// Medias whose content is closest to the one given, best first. Uses stored
    /// vectors only, so it works without the model.
    pub async fn search_similar_medias(
        &self,
        library_id: &str,
        media_id: &str,
        query: MediaEmbeddingSearchQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<MediaEmbeddingSearch> {
        self.media_listing_scope(library_id, Default::default(), requesting_user)
            .await?;
        requesting_user.check_file_role(library_id, media_id, LibraryRole::Read)?;

        let vector = {
            let index = self.embedding_index(library_id).await?;
            index
                .as_ref()
                .and_then(|index| index.get(media_id))
                .map(<[i8]>::to_vec)
        };
        let Some(vector) = vector else {
            let store = self.store.get_library_store(library_id)?;
            if store.get_media_source(media_id).await?.is_none() {
                return Err(ModelError::MediaNotFound(media_id.to_string()).into());
            }
            return Ok(MediaEmbeddingSearch::empty(
                MediaEmbeddingSearchStatus::MediaNotEmbedded,
                "This media has not been embedded yet",
            ));
        };
        self.search_medias_by_vector(
            library_id,
            vector,
            Some(media_id.to_string()),
            query,
            requesting_user,
        )
        .await
    }

    async fn search_medias_by_vector(
        &self,
        library_id: &str,
        vector: Vec<i8>,
        exclude: Option<String>,
        query: MediaEmbeddingSearchQuery,
        requesting_user: &ConnectedUser,
    ) -> RsResult<MediaEmbeddingSearch> {
        let limit = query
            .limit
            .unwrap_or(SEARCH_DEFAULT_LIMIT)
            .min(SEARCH_MAX_LIMIT);
        let offset = query.offset.unwrap_or(0);
        let wanted = offset.saturating_add(limit);
        // The same filters and per-user limits as the media listing.
        let (media_query, limits) = self
            .media_listing_scope(library_id, Default::default(), requesting_user)
            .await?;
        let store = self.store.get_library_store(library_id)?;

        let mut hits: Vec<MediaEmbeddingHit> = Vec::new();
        let mut resolved = 0;
        // Most users can see every ranked media, so a short head of the ranking is
        // enough. Widen it only when access filtering leaves the page short.
        let mut head = wanted
            .saturating_mul(2)
            .saturating_add(SEARCH_RESOLVE_CHUNK);
        loop {
            let (ranked, total) = {
                // The guard lives inside the blocking scan only.
                let index = self.embedding_index(library_id).await?;
                let vector = vector.clone();
                let exclude = exclude.clone();
                tokio::task::spawn_blocking(move || {
                    let Some(index) = index.as_ref() else {
                        return (Vec::new(), 0);
                    };
                    // One more than the head, in case the excluded media is in it.
                    let ranked: Vec<(String, f32)> = index
                        .rank(&vector, head.saturating_add(1))
                        .into_iter()
                        .filter(|(id, _)| exclude.as_deref() != Some(*id))
                        .take(head)
                        .map(|(id, score)| (id.to_string(), score))
                        .collect();
                    (ranked, index.len())
                })
                .await
                .map_err(|error| RsError::Error(format!("Join error: {}", error)))?
            };

            for chunk in ranked[resolved.min(ranked.len())..].chunks(SEARCH_RESOLVE_CHUNK) {
                if hits.len() >= wanted {
                    break;
                }
                let ids = chunk.iter().map(|(id, _)| id.clone()).collect();
                let mut medias: HashMap<String, ItemWithRelations<Media>> = store
                    .get_medias_by_ids(ids, media_query.clone(), limits.clone())
                    .await?
                    .into_iter()
                    .map(|media| (media.item.id.clone(), media))
                    .collect();
                hits.extend(chunk.iter().filter_map(|(id, score)| {
                    medias.remove(id).map(|media| MediaEmbeddingHit {
                        media,
                        score: *score,
                    })
                }));
                resolved += chunk.len();
            }
            if hits.len() >= wanted || head >= total {
                break;
            }
            head = head.saturating_mul(8);
        }

        Ok(MediaEmbeddingSearch {
            status: MediaEmbeddingSearchStatus::Ok,
            embedder: self.embedder_spec().id.to_string(),
            message: None,
            results: hits.into_iter().skip(offset).take(limit).collect(),
        })
    }
}

/// Milliseconds since the epoch, the unit of the embedding failure timestamps.
pub fn embedding_pass_start() -> i64 {
    get_time().as_millis() as i64
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, path::Path};

    use super::*;
    use crate::{
        domain::library::{LibraryType, ServerLibrarySettings},
        model::libraries::ServerLibraryForAdd,
        tools::{scheduler::RsSchedulerTask, video_tools::VideoCommandBuilder},
    };

    async fn status(mc: &ModelController, library_id: &str) -> MediaEmbeddingStatus {
        mc.get_media_embeddings_status(library_id, &ConnectedUser::ServerAdmin)
            .await
            .unwrap()
    }

    fn text_query(text: &str) -> MediaEmbeddingSearchQuery {
        MediaEmbeddingSearchQuery {
            text: Some(text.to_string()),
            ..Default::default()
        }
    }

    /// Whole flow on a throwaway server directory: library opt-in, background task,
    /// text search, similar medias, paging, access check, deletion.
    ///
    /// Downloads the model unless `REDSEAT_EMBEDDING_TEST_DIR` already holds it. Set
    /// `REDSEAT_EMBEDDING_TEST_VIDEO` to a video file to cover videos too (needs FFmpeg,
    /// downloaded next to the working directory when missing).
    /// Run with `cargo test --bin redseat-rust embeds_and_searches_a_library -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "downloads the EmbeddingGemma 2 model and starts a server"]
    async fn embeds_and_searches_a_library() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let directory = tempfile::tempdir().unwrap();
        let config_directory = directory.path().join("config");
        let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        std::env::set_var("REDSEAT_DIR", &config_directory);
        std::env::set_var("REDSEAT_NOCERT", "1");
        std::env::set_var(
            "REDSEAT_PORT",
            listener.local_addr().unwrap().port().to_string(),
        );

        // Reuse an already downloaded model instead of fetching it again.
        if let Ok(models) = std::env::var("REDSEAT_EMBEDDING_TEST_DIR") {
            let target = config_directory
                .join("models")
                .join(embedding::gemma::MODEL_DIRECTORY);
            std::fs::create_dir_all(&target).unwrap();
            for entry in std::fs::read_dir(models).unwrap() {
                let entry = entry.unwrap();
                let destination = target.join(entry.file_name());
                if std::fs::hard_link(entry.path(), &destination).is_err() {
                    std::fs::copy(entry.path(), &destination).unwrap();
                }
            }
        }

        // Building the application router also proves the new routes do not collide.
        let (router, mc) = crate::app().await.unwrap();
        // FFmpeg reads videos through the server's own media endpoint.
        tokio::spawn(async move { axum::serve(listener, router).await });

        let admin = &ConnectedUser::ServerAdmin;
        let library_root = directory.path().join("library");
        std::fs::create_dir_all(&library_root).unwrap();
        let library = mc
            .add_library(
                ServerLibraryForAdd {
                    name: "embeddings".to_string(),
                    source: "PathProvider".to_string(),
                    root: Some(library_root.to_string_lossy().to_string()),
                    settings: ServerLibrarySettings::default(),
                    kind: LibraryType::Photos,
                    crypt: None,
                    credentials: None,
                    plugin: None,
                    password: None,
                },
                None,
                admin,
            )
            .await
            .unwrap()
            .unwrap();
        let library_id = library.id.as_str();

        let cats = reqwest::get(
            "https://huggingface.co/datasets/Xenova/transformers.js-docs/resolve/main/cats.jpg",
        )
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap()
        .to_vec();
        let mut files = vec![("cats.jpg".to_string(), cats)];
        for name in ["image.jpg", "face1.jpg"] {
            files.push((
                name.to_string(),
                std::fs::read(root.join("test_data").join(name)).unwrap(),
            ));
        }
        let video = std::env::var("REDSEAT_EMBEDDING_TEST_VIDEO").ok();
        if let Some(video) = &video {
            if VideoCommandBuilder::version().await.unwrap().is_none() {
                VideoCommandBuilder::download().await.unwrap();
            }
            files.push(("video.mp4".to_string(), std::fs::read(video).unwrap()));
        }
        let mut ids = HashMap::new();
        for (name, bytes) in files {
            let media = mc
                .add_library_file(library_id, &name, None, Cursor::new(bytes), admin)
                .await
                .unwrap();
            println!("added {name} as {} ({:?})", media.id, media.kind);
            ids.insert(name, media.id);
        }
        let total = ids.len() as u64;

        // Off by default: the task leaves the library alone and a manual start is refused.
        let before = status(&mc, library_id).await;
        assert!(!before.enabled);
        assert_eq!(
            (
                before.counts.total,
                before.counts.embedded,
                before.counts.pending
            ),
            (total, 0, total)
        );
        let task = EmbeddingTask {
            specific_library: Some(library_id.to_string()),
        };
        task.execute(mc.clone()).await.unwrap();
        assert_eq!(status(&mc, library_id).await.counts.embedded, 0);
        assert!(mc
            .start_media_embedding_task(library_id, false, admin)
            .await
            .is_err());
        let nothing = mc
            .search_medias_by_text(library_id, text_query("cats"), admin)
            .await
            .unwrap();
        assert_eq!(nothing.status, MediaEmbeddingSearchStatus::NoEmbeddings);
        assert!(nothing.results.is_empty());

        // Opt in through the regular library update, then run the task.
        mc.update_library(
            library_id,
            serde_json::from_value(serde_json::json!({ "settings": { "embeddings": true } }))
                .unwrap(),
            admin,
        )
        .await
        .unwrap();
        assert!(status(&mc, library_id).await.enabled);
        let started = std::time::Instant::now();
        task.execute(mc.clone()).await.unwrap();
        println!("backfill of {total} medias took {:?}", started.elapsed());

        let after = status(&mc, library_id).await;
        println!("{}", serde_json::to_string(&after).unwrap());
        assert_eq!(
            (
                after.counts.embedded,
                after.counts.pending,
                after.counts.failed
            ),
            (total, 0, 0)
        );
        assert_eq!(after.model, EmbeddingModelState::Loaded);
        assert!(!after.running);
        let store = mc.store.get_library_store(library_id).unwrap();
        let stored = store
            .get_media_embedding(ids["cats.jpg"].clone(), after.embedder.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.len(), after.dimension);

        let found = mc
            .search_medias_by_text(library_id, text_query("cats sleeping on a couch"), admin)
            .await
            .unwrap();
        for hit in &found.results {
            println!("cats query: {} {:.4}", hit.media.item.name, hit.score);
        }
        assert_eq!(found.status, MediaEmbeddingSearchStatus::Ok);
        assert_eq!(found.results.len() as u64, total);
        assert_eq!(found.results[0].media.item.id, ids["cats.jpg"]);
        assert!(found
            .results
            .windows(2)
            .all(|pair| pair[0].score >= pair[1].score));
        println!("{}", serde_json::to_string(&found.results[0]).unwrap());

        let page = mc
            .search_medias_by_text(
                library_id,
                MediaEmbeddingSearchQuery {
                    text: Some("cats sleeping on a couch".to_string()),
                    limit: Some(1),
                    offset: Some(1),
                },
                admin,
            )
            .await
            .unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(
            page.results[0].media.item.id,
            found.results[1].media.item.id
        );

        let similar = mc
            .search_similar_medias(library_id, &ids["cats.jpg"], Default::default(), admin)
            .await
            .unwrap();
        for hit in &similar.results {
            println!("similar to cats: {} {:.4}", hit.media.item.name, hit.score);
        }
        assert_eq!(similar.status, MediaEmbeddingSearchStatus::Ok);
        assert_eq!(similar.results.len() as u64, total - 1);
        assert!(similar
            .results
            .iter()
            .all(|hit| hit.media.item.id != ids["cats.jpg"]));

        // No library access, no results.
        assert!(mc
            .search_medias_by_text(library_id, text_query("cats"), &ConnectedUser::Anonymous)
            .await
            .is_err());
        assert!(mc
            .search_similar_medias(
                library_id,
                &ids["cats.jpg"],
                Default::default(),
                &ConnectedUser::Anonymous
            )
            .await
            .is_err());
        assert!(mc
            .get_media_embeddings_status(library_id, &ConnectedUser::Anonymous)
            .await
            .is_err());

        // A deleted media leaves the index and the table.
        mc.remove_media(library_id, &ids["cats.jpg"], admin)
            .await
            .unwrap();
        let found = mc
            .search_medias_by_text(library_id, text_query("cats sleeping on a couch"), admin)
            .await
            .unwrap();
        assert_eq!(found.results.len() as u64, total - 1);
        assert!(found
            .results
            .iter()
            .all(|hit| hit.media.item.id != ids["cats.jpg"]));
        assert!(store
            .get_media_embedding(ids["cats.jpg"].clone(), after.embedder.clone())
            .await
            .unwrap()
            .is_none());
        assert_eq!(status(&mc, library_id).await.counts.total, total - 1);
    }
}
