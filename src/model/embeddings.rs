use std::{
    collections::{HashMap, HashSet},
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
    error::Error as ModelError,
    medias::MediaQuery,
    store::sql::library::{
        embeddings::MediaEmbeddingCounts, medias::MEDIA_QUERY_DEFAULT_LIMIT, SqliteLibraryStore,
    },
    users::ConnectedUser,
    ModelController,
};
use crate::{
    domain::{
        library::{LibraryLimits, LibraryRole, ServerLibrarySettingsPatch},
        media::{FileType, Media},
    },
    error::{RsError, RsResult},
    plugins::sources::error::SourcesError,
    routes::sse::SseEvent,
    server::get_server_local_path,
    tools::{
        auth::ClaimsLocalType,
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
/// How far down the ranking a page may reach (`offset + limit`). A page is found by
/// resolving the ranking from its top, so this bounds the work of one request.
const SEARCH_MAX_WINDOW: usize = 10_000;
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
    /// Library setting `embeddingsPaused`: nothing is embedded until it is resumed.
    pub paused: bool,
    pub model: EmbeddingModelState,
    /// A backfill is running on this server (any library).
    pub running: bool,
    #[serde(flatten)]
    pub counts: MediaEmbeddingCounts,
}

/// Where a background embedding pass over one library stands.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum EmbeddingPassState {
    /// Sent before the model is loaded and, the first time, downloaded.
    Started,
    /// Sent after each media.
    Running,
    Completed,
    /// Left because the library was paused; resuming starts a new pass.
    Paused,
    /// Left because embeddings were turned off for the library, or it was removed.
    Stopped,
    /// Left early after too many failures in a row; a later pass carries on.
    Interrupted,
    /// Could not run, for example because the model cannot be loaded.
    Failed,
}

/// Payload of the `embedding_progress` event. A pass sends `started` once, `running`
/// after each media, then exactly one of `completed`, `paused`, `stopped`,
/// `interrupted` or `failed`.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingProgressMessage {
    pub library: String,
    pub state: EmbeddingPassState,
    pub embedder: String,
    /// Medias tried so far in this pass.
    pub processed: u64,
    /// Of which embedded; the others failed.
    pub embedded: u64,
    /// Medias this pass covers: those pending when it started, kept in step with
    /// uploads and deletions since. Never below `processed`.
    pub total: u64,
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

impl MediaEmbeddingSearchQuery {
    /// `(limit, offset)` of the requested page. Refused when it reaches further down
    /// the ranking than `SEARCH_MAX_WINDOW`.
    fn page(&self) -> RsResult<(usize, usize)> {
        let limit = self
            .limit
            .unwrap_or(SEARCH_DEFAULT_LIMIT)
            .min(SEARCH_MAX_LIMIT);
        let offset = self.offset.unwrap_or(0);
        if offset.saturating_add(limit) > SEARCH_MAX_WINDOW {
            return Err(RsError::InvalidParams(format!(
                "offset + limit cannot exceed {SEARCH_MAX_WINDOW}"
            )));
        }
        Ok((limit, offset))
    }
}

/// Search, status and progress cover the whole library, which a token scoped to a
/// single file must not reach: `check_library_role` accepts such a token for its
/// library. The other share claims are admin credentials, or are refused by the
/// role checks themselves.
pub(crate) fn check_library_wide_access(requesting_user: &ConnectedUser) -> RsResult<()> {
    if let ConnectedUser::Share(claims) = requesting_user {
        if matches!(claims.kind, ClaimsLocalType::File(..)) {
            return Err(ModelError::ShareTokenInsufficient.into());
        }
    }
    Ok(())
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

    /// Loads the embedder with its image side. A pass calls it first, so a model that
    /// cannot load stops the pass before any media is charged with the failure.
    pub async fn prepare_embedder_for_images(&self) -> RsResult<()> {
        let embedder = self.get_embedder().await?;
        tokio::task::spawn_blocking(move || embedder.prepare_image_resources())
            .await
            .map_err(|error| RsError::Error(format!("Join error: {}", error)))??;
        Ok(())
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

    pub fn send_embedding_progress(&self, message: EmbeddingProgressMessage) {
        self.broadcast_sse(SseEvent::EmbeddingProgress(message));
    }

    /// Whether background embedding is turned on for the library (off by default).
    pub async fn library_embeddings_enabled(&self, library_id: &str) -> bool {
        self.cache_get_library(library_id)
            .await
            .and_then(|library| library.settings.embeddings)
            .unwrap_or(false)
    }

    /// Whether background embedding is suspended for the library.
    pub async fn library_embeddings_paused(&self, library_id: &str) -> bool {
        self.cache_get_library(library_id)
            .await
            .and_then(|library| library.settings.embeddings_paused)
            .unwrap_or(false)
    }

    /// Suspends or resumes background embedding on a library. A running pass leaves
    /// the library once the media in progress is done. Resuming queues a pass right
    /// away. The flag is a library setting, so it survives a restart.
    pub async fn set_media_embeddings_paused(
        &self,
        library_id: &str,
        paused: bool,
        requesting_user: &ConnectedUser,
    ) -> RsResult<MediaEmbeddingStatus> {
        self.update_library_settings(
            library_id,
            ServerLibrarySettingsPatch {
                embeddings_paused: Some(paused.then_some(true)),
                ..Default::default()
            },
            requesting_user,
        )
        .await?;
        if !paused
            && self.library_embeddings_enabled(library_id).await
            && !self.cache_get_library_crypt(library_id).await
        {
            self.scheduler
                .add(
                    RsTaskType::Embedding,
                    RsSchedulerWhen::At(0),
                    EmbeddingTask {
                        specific_library: Some(library_id.to_string()),
                    },
                )
                .await?;
        }
        self.get_media_embeddings_status(library_id, requesting_user)
            .await
    }

    pub async fn get_media_embeddings_status(
        &self,
        library_id: &str,
        requesting_user: &ConnectedUser,
    ) -> RsResult<MediaEmbeddingStatus> {
        check_library_wide_access(requesting_user)?;
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
            paused: self.library_embeddings_paused(library_id).await,
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

    /// How many medias a pass started at `pass_started` (ms) still has to try.
    pub async fn count_medias_left_in_embedding_pass(
        &self,
        library_id: &str,
        pass_started: i64,
    ) -> RsResult<u64> {
        let store = self.store.get_library_store(library_id)?;
        let left = store
            .count_medias_pending_embedding(
                self.embedder_spec().id.to_string(),
                EMBEDDING_MAX_ATTEMPTS,
                pass_started,
            )
            .await?;
        Ok(left)
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
    ///
    /// Returns false when a pass is already running. Passes do not overlap, so the
    /// library is then handled by that pass, or by the next recurring one if that
    /// pass is already past it.
    pub async fn start_media_embedding_task(
        &self,
        library_id: &str,
        retry_failed: bool,
        requesting_user: &ConnectedUser,
    ) -> RsResult<bool> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        self.cache_check_library_notcrypt(library_id).await?;
        if !self.library_embeddings_enabled(library_id).await {
            return Err(RsError::InvalidParams(
                "Embeddings are not enabled for this library".to_string(),
            ));
        }
        if self.library_embeddings_paused(library_id).await {
            return Err(RsError::InvalidParams(
                "Embeddings are paused for this library".to_string(),
            ));
        }
        if retry_failed {
            let store = self.store.get_library_store(library_id)?;
            store
                .clear_media_embedding_errors(self.embedder_spec().id.to_string())
                .await?;
        }
        let already_running = self.embeddings.backfill_running.load(Ordering::Relaxed);
        // Queued even then: the running pass may be over before the scheduler ticks.
        self.scheduler
            .add(
                RsTaskType::Embedding,
                RsSchedulerWhen::At(0),
                EmbeddingTask {
                    specific_library: Some(library_id.to_string()),
                },
            )
            .await?;
        Ok(!already_running)
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
                // Held across the write, so the row and the index change together: a
                // delete of this media removes it from the index only once this is
                // done, and an index load reads the row either before or after it.
                let slot = self.embedding_index_slot(library_id).await;
                let mut index = slot.write().await;
                let stored = store
                    .add_media_embedding(
                        media_id.to_string(),
                        spec.id.to_string(),
                        embedding::to_bytes(&quantized),
                    )
                    .await?;
                // Not stored when the media was deleted while its vector was computed.
                if stored {
                    if let Some(index) = &mut *index {
                        index.upsert(media_id, &quantized);
                    }
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
        check_library_wide_access(requesting_user)?;
        self.media_listing_scope(library_id, Default::default(), requesting_user)
            .await?;
        // Before the query is embedded.
        query.page()?;
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
        let Some(vector) = self.embed_search_text(text).await? else {
            return Ok(MediaEmbeddingSearch::empty(
                MediaEmbeddingSearchStatus::ModelDownloading,
                "The embedding model is being downloaded, try again later",
            ));
        };
        self.search_medias_by_vector(library_id, vector, None, query, requesting_user)
            .await
    }

    /// The vector of a search text, ready to be compared with the stored ones.
    /// `None` when the model is not on disk yet.
    async fn embed_search_text(&self, text: String) -> RsResult<Option<Vec<i8>>> {
        let Some(embedder) = self.get_embedder_for_search().await? else {
            return Ok(None);
        };
        let scale = embedder.spec().quantization_scale;
        let vector = tokio::task::spawn_blocking(move || embedder.embed_text(&text))
            .await
            .map_err(|error| RsError::Error(format!("Join error: {}", error)))??;
        Ok(Some(embedding::quantize(&vector, scale)))
    }

    /// The listing of a text search, ranked by content. First the medias that match
    /// the text the way the listing does, closest content first. Then the other
    /// medias the rest of the query allows, closest content first.
    ///
    /// `None` when the listing has to answer as it does without embeddings: no text,
    /// nothing embedded in the library, or no model to embed the text with.
    pub(super) async fn search_medias_hybrid(
        &self,
        library_id: &str,
        query: &MediaQuery,
        limits: &LibraryLimits,
        requesting_user: &ConnectedUser,
    ) -> RsResult<Option<Vec<ItemWithRelations<Media>>>> {
        let Some(text) = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            return Ok(None);
        };
        if check_library_wide_access(requesting_user).is_err() {
            return Ok(None);
        }
        // The ranking is resumed after the media a page key names. A key that names
        // none can only page the plain listing.
        let after = match query.page_key.as_deref() {
            None => None,
            Some(key) => match key.split_once('|') {
                Some((_, id)) => Some(id.to_string()),
                None => return Ok(None),
            },
        };
        if self
            .embedding_index(library_id)
            .await?
            .as_ref()
            .is_none_or(VectorIndex::is_empty)
        {
            return Ok(None);
        }
        let vector = match self.embed_search_text(text.to_string()).await {
            Ok(Some(vector)) => vector,
            Ok(None) => return Ok(None),
            Err(error) => {
                // The listing still answers when the model does not.
                log_error(
                    LogServiceType::Source,
                    format!(
                        "Unable to embed the search text, listing exact matches only: {:#}",
                        error
                    ),
                );
                return Ok(None);
            }
        };

        let store = self.store.get_library_store(library_id)?;
        let index = self.embedding_index_slot(library_id).await;
        let medias = hybrid_listing(&store, index, vector, query, limits, after).await?;
        Ok(Some(medias))
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
        check_library_wide_access(requesting_user)?;
        let (media_query, limits) = self
            .media_listing_scope(library_id, Default::default(), requesting_user)
            .await?;
        requesting_user.check_file_role(library_id, media_id, LibraryRole::Read)?;
        // The reference goes through the listing scope like the results: a media the
        // user cannot list must not be usable as one, even when its id is known.
        let store = self.store.get_library_store(library_id)?;
        let listed = store
            .get_medias_by_ids(vec![media_id.to_string()], media_query, limits)
            .await?;
        if listed.is_empty() {
            return Err(ModelError::MediaNotFound(media_id.to_string()).into());
        }

        let vector = {
            let index = self.embedding_index(library_id).await?;
            index
                .as_ref()
                .and_then(|index| index.get(media_id))
                .map(<[i8]>::to_vec)
        };
        let Some(vector) = vector else {
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
        let (limit, offset) = query.page()?;
        let wanted = offset + limit;
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

/// One page of a text search over the listing, in the order of [`hybrid_order`].
/// `vector` is the text as the index compares it, `after` the last media of the
/// previous page.
async fn hybrid_listing(
    store: &SqliteLibraryStore,
    index: IndexSlot,
    vector: Vec<i8>,
    query: &MediaQuery,
    limits: &LibraryLimits,
    after: Option<String>,
) -> RsResult<Vec<ItemWithRelations<Media>>> {
    let limit = query.limit.unwrap_or(MEDIA_QUERY_DEFAULT_LIMIT);
    let mut exact_query = query.clone();
    exact_query.page_key = None;
    // Without the text, the query says which medias may be returned at all.
    let mut scope_query = exact_query.clone();
    scope_query.text = None;
    let scope = store
        .get_media_ids(scope_query.clone(), limits.clone())
        .await?;
    let exact: HashSet<String> = store
        .get_media_ids(exact_query, limits.clone())
        .await?
        .into_iter()
        .collect();

    // The guard lives inside the blocking scan only.
    let index = index.read_owned().await;
    let page = tokio::task::spawn_blocking(move || {
        let scores = match index.as_ref() {
            Some(index) => index.scores(&vector, &scope),
            None => vec![None; scope.len()],
        };
        page_after(
            hybrid_order(scope, &exact, &scores),
            after.as_deref(),
            limit,
        )
    })
    .await
    .map_err(|error| RsError::Error(format!("Join error: {}", error)))?;

    let mut medias = Vec::with_capacity(page.len());
    for chunk in page.chunks(SEARCH_RESOLVE_CHUNK) {
        let mut found: HashMap<String, ItemWithRelations<Media>> = store
            .get_medias_by_ids(chunk.to_vec(), scope_query.clone(), limits.clone())
            .await?
            .into_iter()
            .map(|media| (media.item.id.clone(), media))
            .collect();
        medias.extend(chunk.iter().filter_map(|id| found.remove(id)));
    }
    Ok(medias)
}

/// Order of a text search over `scope`, the medias the rest of the query allows, in
/// the order of the listing. `scores` gives the similarity of each with the text,
/// `None` when it has no vector.
///
/// The medias of `exact` come first: closest content first, then those without a
/// vector in the order of the listing. The other medias that have a vector follow,
/// closest first. Equal scores are ordered by id so paging is stable.
fn hybrid_order(
    scope: Vec<String>,
    exact: &HashSet<String>,
    scores: &[Option<f32>],
) -> Vec<String> {
    let mut exact_ranked = Vec::new();
    let mut exact_unranked = Vec::new();
    let mut others = Vec::new();
    for (id, score) in scope.into_iter().zip(scores) {
        match (exact.contains(&id), score) {
            (true, Some(score)) => exact_ranked.push((id, *score)),
            (true, None) => exact_unranked.push(id),
            (false, Some(score)) => others.push((id, *score)),
            (false, None) => {}
        }
    }
    let closest_first =
        |a: &(String, f32), b: &(String, f32)| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0));
    exact_ranked.sort_unstable_by(closest_first);
    others.sort_unstable_by(closest_first);
    exact_ranked
        .into_iter()
        .map(|(id, _)| id)
        .chain(exact_unranked)
        .chain(others.into_iter().map(|(id, _)| id))
        .collect()
}

/// The `limit` ids that follow `after` in `order`, from its start when there is none.
/// Nothing when `after` is no longer in it: the page cannot be placed.
fn page_after(order: Vec<String>, after: Option<&str>, limit: usize) -> Vec<String> {
    let start = match after {
        None => 0,
        Some(after) => match order.iter().position(|id| id == after) {
            Some(position) => position + 1,
            None => return Vec::new(),
        },
    };
    order.into_iter().skip(start).take(limit).collect()
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
        domain::{
            library::{LibraryLimits, LibraryType, ServerLibrarySettings},
            media::MediaForAdd,
        },
        model::{
            libraries::ServerLibraryForAdd,
            users::{ServerUser, ServerUserLibrariesRights, UserRole},
        },
        tools::{
            auth::{ClaimsLocal, ClaimsLocalType},
            scheduler::RsSchedulerTask,
            video_tools::VideoCommandBuilder,
        },
    };

    async fn status(mc: &ModelController, library_id: &str) -> MediaEmbeddingStatus {
        mc.get_media_embeddings_status(library_id, &ConnectedUser::ServerAdmin)
            .await
            .unwrap()
    }

    /// The next `embedding_progress` event of the library.
    async fn next_progress(
        events: &mut tokio::sync::broadcast::Receiver<SseEvent>,
        library_id: &str,
    ) -> EmbeddingProgressMessage {
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(300), events.recv())
                .await
                .expect("no embedding progress in time")
                .unwrap();
            if let SseEvent::EmbeddingProgress(message) = event {
                if message.library == library_id {
                    return message;
                }
            }
        }
    }

    fn in_progress(state: EmbeddingPassState) -> bool {
        matches!(
            state,
            EmbeddingPassState::Started | EmbeddingPassState::Running
        )
    }

    fn summary(pass: &[EmbeddingProgressMessage]) -> Vec<(EmbeddingPassState, u64, u64)> {
        pass.iter()
            .map(|message| (message.state, message.processed, message.embedded))
            .collect()
    }

    /// The last event of a pass is sent just before the pass is marked as over.
    async fn wait_idle(mc: &ModelController, library_id: &str) {
        while status(mc, library_id).await.running {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    #[test]
    fn search_page_is_bounded() {
        let page = |limit, offset| {
            MediaEmbeddingSearchQuery {
                text: None,
                limit,
                offset,
            }
            .page()
        };
        assert_eq!(page(None, None).unwrap(), (SEARCH_DEFAULT_LIMIT, 0));
        // The limit is capped, the offset is not: it is refused past the window.
        assert_eq!(
            page(Some(usize::MAX), Some(7)).unwrap(),
            (SEARCH_MAX_LIMIT, 7)
        );
        assert!(page(Some(50), Some(SEARCH_MAX_WINDOW - 50)).is_ok());
        assert!(page(Some(50), Some(SEARCH_MAX_WINDOW - 49)).is_err());
        assert!(page(Some(0), Some(usize::MAX)).is_err());
    }

    fn text_query(text: &str) -> MediaEmbeddingSearchQuery {
        MediaEmbeddingSearchQuery {
            text: Some(text.to_string()),
            ..Default::default()
        }
    }

    fn listing_query(text: &str) -> MediaQuery {
        MediaQuery {
            text: Some(text.to_string()),
            ..Default::default()
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn hybrid_order_puts_text_matches_first_then_the_closest_others() {
        let scope = strings(&["a", "b", "c", "d", "e", "f", "g"]);
        let exact: HashSet<String> = strings(&["b", "d", "e", "g"]).into_iter().collect();
        let scores = [Some(0.9), Some(0.2), Some(0.5), None, Some(0.7), None, None];
        // "e" and "b" match the text and have a vector, "d" and "g" match without
        // one and keep the order of the listing. "a" and "c" only have a vector,
        // "f" has neither.
        assert_eq!(
            hybrid_order(scope, &exact, &scores),
            strings(&["e", "b", "d", "g", "a", "c"])
        );

        // Equal scores fall back to the id, whatever the order of the listing.
        let tied = hybrid_order(
            strings(&["z", "y", "x"]),
            &HashSet::new(),
            &[Some(0.5), Some(0.5), Some(0.6)],
        );
        assert_eq!(tied, strings(&["x", "y", "z"]));
    }

    #[test]
    fn page_after_resumes_from_the_last_media() {
        let order = strings(&["a", "b", "c", "d"]);
        assert_eq!(
            page_after(order.clone(), None, 3),
            strings(&["a", "b", "c"])
        );
        assert_eq!(page_after(order.clone(), Some("c"), 3), strings(&["d"]));
        assert!(page_after(order.clone(), Some("d"), 3).is_empty());
        // A media that left the ranking cannot place the page.
        assert!(page_after(order.clone(), Some("gone"), 3).is_empty());
        assert!(page_after(order, None, 0).is_empty());
    }

    /// The ranking of a text search on a real store: the criteria and the limits of
    /// the user still decide what is returned, pages follow the last media.
    #[tokio::test]
    async fn hybrid_listing_keeps_the_criteria_and_pages_by_last_media() {
        async fn listed(
            store: &SqliteLibraryStore,
            index: &IndexSlot,
            query: MediaQuery,
            limits: LibraryLimits,
            after: Option<&str>,
        ) -> Vec<String> {
            hybrid_listing(
                store,
                index.clone(),
                vec![100, 0],
                &query,
                &limits,
                after.map(str::to_string),
            )
            .await
            .unwrap()
            .into_iter()
            .map(|media| media.item.id)
            .collect()
        }

        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        for (id, name, description, kind) in [
            ("sea", "IMG_0001.jpg", None, FileType::Photo),
            ("named", "beach.jpg", None, FileType::Photo),
            (
                "described",
                "IMG_0003.jpg",
                Some("a day at the beach"),
                FileType::Photo,
            ),
            ("clip", "beach.mp4", None, FileType::Video),
            ("city", "IMG_0005.jpg", None, FileType::Photo),
            ("archive", "beach.zip", None, FileType::Archive),
            ("pending", "IMG_0007.jpg", None, FileType::Photo),
        ] {
            store
                .add_media(
                    MediaForAdd {
                        name: name.to_string(),
                        description: description.map(str::to_string),
                        kind,
                        ..Default::default()
                    }
                    .into_insert_with_id(id.to_string()),
                )
                .await
                .unwrap();
        }
        // Closeness to the query `[100, 0]`: sea, clip, described, named, city.
        // "archive" and "pending" have no vector.
        let mut index = VectorIndex::new(2);
        for (id, vector) in [
            ("sea", [100, 5]),
            ("named", [20, 100]),
            ("described", [80, 60]),
            ("clip", [90, 30]),
            ("city", [-100, 10]),
        ] {
            index.upsert(id, &vector);
        }
        let index: IndexSlot = Arc::new(RwLock::new(Some(index)));
        let everyone = LibraryLimits::default;

        // Name and description matches first, the one without a vector last among
        // them, then the rest of the library by content. "pending" is left out.
        let all = listed(&store, &index, listing_query("beach"), everyone(), None).await;
        assert_eq!(
            all,
            strings(&["clip", "described", "named", "archive", "sea", "city"])
        );

        // A text matching nothing is a search by content alone.
        let by_content = listed(&store, &index, listing_query("sunset"), everyone(), None).await;
        assert_eq!(
            by_content,
            strings(&["sea", "clip", "described", "named", "city"])
        );

        // The other criteria apply to both groups.
        let photos = MediaQuery {
            types: vec![FileType::Photo],
            ..listing_query("beach")
        };
        assert_eq!(
            listed(&store, &index, photos, everyone(), None).await,
            strings(&["described", "named", "sea", "city"])
        );

        // Pages are cut in the same ranking, each one after the last media of the
        // previous one, across both groups.
        let page = MediaQuery {
            limit: Some(2),
            ..listing_query("beach")
        };
        let mut paged = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let ids = listed(&store, &index, page.clone(), everyone(), after.as_deref()).await;
            if ids.is_empty() {
                break;
            }
            assert!(ids.len() <= 2);
            after = ids.last().cloned();
            paged.extend(ids);
        }
        assert_eq!(paged, all);

        // A user whose grant hides recent medias (everything here) gets nothing.
        let delayed = LibraryLimits {
            delay: Some(60),
            ..Default::default()
        };
        assert!(
            listed(&store, &index, listing_query("beach"), delayed, None)
                .await
                .is_empty()
        );

        // Without an index only the text matches are left, in the listing's order.
        let no_index: IndexSlot = Default::default();
        let plain = store
            .get_media_ids(listing_query("beach"), everyone())
            .await
            .unwrap();
        assert_eq!(plain.len(), 4);
        assert_eq!(
            listed(&store, &no_index, listing_query("beach"), everyone(), None).await,
            plain
        );
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
        // A fourth one, so a pass can be cut short twice and still leave work.
        let mut plain = Cursor::new(Vec::new());
        image::RgbImage::from_pixel(640, 480, image::Rgb([200, 40, 40]))
            .write_to(&mut plain, image::ImageFormat::Png)
            .unwrap();
        files.push(("plain.png".to_string(), plain.into_inner()));
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
        // With nothing embedded, a text on the listing is the plain filter.
        let plain = mc
            .get_medias(library_id, listing_query("cats"), admin)
            .await
            .unwrap();
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].item.id, ids["cats.jpg"]);

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

        // Paused before anything ran: the task leaves the library alone and a manual
        // start is refused. The flag is in the database, beside the other settings.
        let paused = mc
            .set_media_embeddings_paused(library_id, true, admin)
            .await
            .unwrap();
        assert!(paused.paused && paused.enabled);
        task.execute(mc.clone()).await.unwrap();
        assert_eq!(status(&mc, library_id).await.counts.embedded, 0);
        assert!(mc
            .start_media_embedding_task(library_id, false, admin)
            .await
            .is_err());
        let saved = mc
            .store
            .get_library(library_id)
            .await
            .unwrap()
            .unwrap()
            .settings;
        assert_eq!(
            (saved.embeddings, saved.embeddings_paused),
            (Some(true), Some(true))
        );

        // Resuming queues a pass. Paused again after its first media, that pass
        // leaves the library once the media in progress is done.
        let mut events = mc.sse_tx.subscribe();
        let started = std::time::Instant::now();
        let resumed = mc
            .set_media_embeddings_paused(library_id, false, admin)
            .await
            .unwrap();
        assert!(!resumed.paused);
        let mut first_pass = Vec::new();
        loop {
            let message = next_progress(&mut events, library_id).await;
            let state = message.state;
            first_pass.push(message);
            if state == EmbeddingPassState::Running && first_pass.len() == 2 {
                mc.set_media_embeddings_paused(library_id, true, admin)
                    .await
                    .unwrap();
            }
            if !in_progress(state) {
                break;
            }
        }
        println!("first pass: {:?}", summary(&first_pass));
        assert_eq!(first_pass[0].state, EmbeddingPassState::Started);
        assert_eq!(first_pass[0].total, total);
        let last = &first_pass[first_pass.len() - 1];
        assert_eq!(last.state, EmbeddingPassState::Paused);
        let done = last.processed;
        assert!(done >= 1 && done < total, "paused after {done} of {total}");
        assert_eq!(last.embedded, done);
        wait_idle(&mc, library_id).await;
        let partial = status(&mc, library_id).await;
        assert!(partial.paused);
        assert_eq!(
            (partial.counts.embedded, partial.counts.pending),
            (done, total - done)
        );

        // Resumed, then embeddings are turned off as the new pass starts: it leaves
        // the library and says it was stopped, not that it completed.
        mc.set_media_embeddings_paused(library_id, false, admin)
            .await
            .unwrap();
        let mut second_pass = Vec::new();
        loop {
            let message = next_progress(&mut events, library_id).await;
            let state = message.state;
            second_pass.push(message);
            if state == EmbeddingPassState::Started {
                mc.update_library_settings(
                    library_id,
                    ServerLibrarySettingsPatch {
                        embeddings: Some(Some(false)),
                        ..Default::default()
                    },
                    admin,
                )
                .await
                .unwrap();
            }
            if !in_progress(state) {
                break;
            }
        }
        println!("second pass: {:?}", summary(&second_pass));
        let last = &second_pass[second_pass.len() - 1];
        assert_eq!(last.state, EmbeddingPassState::Stopped);
        let left = total - done - last.processed;
        assert!(left >= 1, "nothing left after the stopped pass");
        wait_idle(&mc, library_id).await;
        assert!(!status(&mc, library_id).await.enabled);

        // Turned back on and started by hand: the pass embeds what is left and
        // reports itself with started, one event per media, then completed.
        mc.update_library_settings(
            library_id,
            ServerLibrarySettingsPatch {
                embeddings: Some(Some(true)),
                ..Default::default()
            },
            admin,
        )
        .await
        .unwrap();
        assert!(mc
            .start_media_embedding_task(library_id, false, admin)
            .await
            .unwrap());
        let mut third_pass = Vec::new();
        loop {
            let message = next_progress(&mut events, library_id).await;
            let state = message.state;
            third_pass.push(message);
            if state == EmbeddingPassState::Started {
                // Uploaded while the pass runs: embedded by it, and counted in.
                let mut late = Cursor::new(Vec::new());
                image::RgbImage::from_pixel(640, 480, image::Rgb([40, 40, 200]))
                    .write_to(&mut late, image::ImageFormat::Png)
                    .unwrap();
                let media = mc
                    .add_library_file(
                        library_id,
                        "late.png",
                        None,
                        Cursor::new(late.into_inner()),
                        admin,
                    )
                    .await
                    .unwrap();
                ids.insert("late.png".to_string(), media.id);
            }
            if !in_progress(state) {
                break;
            }
        }
        println!(
            "third pass: {:?}, totals {:?}",
            summary(&third_pass),
            third_pass.iter().map(|m| m.total).collect::<Vec<_>>()
        );
        // The total starts at what was pending and follows the upload: progress
        // never runs past it, and it ends equal to what was done.
        assert_eq!(third_pass[0].total, left);
        assert!(third_pass.iter().all(|m| m.processed <= m.total));
        let (left, total) = (left + 1, total + 1);
        assert_eq!(third_pass[third_pass.len() - 1].total, left);
        let mut expected = vec![(EmbeddingPassState::Started, 0, 0)];
        expected.extend((1..=left).map(|n| (EmbeddingPassState::Running, n, n)));
        expected.push((EmbeddingPassState::Completed, left, left));
        assert_eq!(summary(&third_pass), expected);
        println!(
            "backfill of {total} medias took {:?}, pause and stop included",
            started.elapsed()
        );
        wait_idle(&mc, library_id).await;

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

        // The listing ranks a text the same way. No name holds this one, so the
        // whole library comes back in the order of the search by content.
        let listed = mc
            .get_medias(library_id, listing_query("cats sleeping on a couch"), admin)
            .await
            .unwrap();
        assert_eq!(
            listed.iter().map(|m| &m.item.id).collect::<Vec<_>>(),
            found
                .results
                .iter()
                .map(|hit| &hit.media.item.id)
                .collect::<Vec<_>>()
        );
        // A name that holds the text comes first, the rest still follows.
        let named = mc
            .get_medias(library_id, listing_query("image"), admin)
            .await
            .unwrap();
        for media in &named {
            println!("image query: {}", media.item.name);
        }
        assert_eq!(named.len() as u64, total);
        assert_eq!(named[0].item.id, ids["image.jpg"]);
        // The next page starts after the media the page key names.
        let next = mc
            .get_medias(
                library_id,
                MediaQuery {
                    limit: Some(1),
                    page_key: Some(format!("0|{}", named[0].item.id)),
                    ..listing_query("image")
                },
                admin,
            )
            .await
            .unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].item.id, named[1].item.id);
        // The other criteria still decide what may be returned.
        let archives = mc
            .get_medias(
                library_id,
                MediaQuery {
                    types: vec![FileType::Archive],
                    ..listing_query("image")
                },
                admin,
            )
            .await
            .unwrap();
        assert!(archives.is_empty());
        // Without a text the listing is untouched.
        let untouched = mc
            .get_medias(library_id, Default::default(), admin)
            .await
            .unwrap();
        assert_eq!(untouched.len() as u64, total);

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

        // A share token for one file reaches nothing beyond that file: not the
        // search, not medias similar to it or to another one, not the status.
        let share = &ConnectedUser::Share(ClaimsLocal {
            cr: "test".to_string(),
            kind: ClaimsLocalType::File(library_id.to_string(), ids["cats.jpg"].clone()),
            exp: ClaimsLocal::generate_seconds(60),
        });
        assert!(mc
            .search_medias_by_text(library_id, text_query("cats"), share)
            .await
            .is_err());
        for reference in ["cats.jpg", "image.jpg"] {
            assert!(mc
                .search_similar_medias(library_id, &ids[reference], Default::default(), share)
                .await
                .is_err());
        }
        assert!(mc
            .get_media_embeddings_status(library_id, share)
            .await
            .is_err());

        // A local admin credential is a share claim too, and is not refused: it
        // reads the status and gets it back from a pause and a resume.
        let local_admin = &ConnectedUser::Share(ClaimsLocal {
            cr: "test".to_string(),
            kind: ClaimsLocalType::Admin,
            exp: ClaimsLocal::generate_seconds(60),
        });
        assert!(mc
            .get_media_embeddings_status(library_id, local_admin)
            .await
            .is_ok());
        let paused = mc
            .set_media_embeddings_paused(library_id, true, local_admin)
            .await
            .unwrap();
        assert!(paused.paused);
        let resumed = mc
            .set_media_embeddings_paused(library_id, false, local_admin)
            .await
            .unwrap();
        assert!(!resumed.paused);

        // A page too far down the ranking is refused before any work is done.
        for (limit, offset) in [(0, usize::MAX), (50, 10_000)] {
            let too_deep = MediaEmbeddingSearchQuery {
                text: Some("cats".to_string()),
                limit: Some(limit),
                offset: Some(offset),
            };
            assert!(mc
                .search_medias_by_text(library_id, too_deep.clone(), admin)
                .await
                .is_err());
            assert!(mc
                .search_similar_medias(library_id, &ids["cats.jpg"], too_deep, admin)
                .await
                .is_err());
        }

        // A user whose grant hides recent medias (everything here) finds nothing,
        // and cannot use a hidden media as a similarity reference either.
        let delayed = &ConnectedUser::Server(ServerUser {
            id: "delayed".to_string(),
            name: "delayed".to_string(),
            role: UserRole::Read,
            preferences: Default::default(),
            libraries: vec![ServerUserLibrariesRights {
                id: library_id.to_string(),
                name: "embeddings".to_string(),
                kind: LibraryType::Photos,
                roles: vec![LibraryRole::Read],
                limits: LibraryLimits {
                    delay: Some(60),
                    ..Default::default()
                },
            }],
        });
        let hidden = mc
            .search_medias_by_text(library_id, text_query("cats"), delayed)
            .await
            .unwrap();
        assert!(hidden.results.is_empty());
        assert!(mc
            .search_similar_medias(library_id, &ids["cats.jpg"], Default::default(), delayed)
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
