use crate::{
    error::RsResult,
    model::{
        embeddings::{embedding_pass_start, EmbeddingPassState, EmbeddingProgressMessage},
        users::ConnectedUser,
        ModelController,
    },
    tools::log::{log_error, log_info, LogServiceType},
};
use axum::async_trait;
use serde::{Deserialize, Serialize};

use super::RsSchedulerTask;

/// Embeds the photos and videos that have no vector yet for the active embedder,
/// in the libraries that turned embeddings on.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EmbeddingTask {
    pub specific_library: Option<String>,
}

impl EmbeddingTask {
    pub fn new() -> Self {
        Self {
            specific_library: None,
        }
    }
}

/// Counters of a pass over one library. Kept by the caller, so they are known
/// however the pass ends.
struct LibraryPass {
    library: String,
    embedder: String,
    /// Medias the pass covers, see `EmbeddingProgressMessage::total`.
    total: u64,
    processed: u64,
    embedded: u64,
    /// Set when the pass leaves the library before its pending medias are done.
    stopped: Option<EmbeddingPassState>,
}

/// Why a pass must leave the library now, as the state to report, if it must.
async fn must_stop(mc: &ModelController, library_id: &str) -> Option<EmbeddingPassState> {
    if !mc.library_embeddings_enabled(library_id).await {
        // Embeddings turned off, or the library removed.
        Some(EmbeddingPassState::Stopped)
    } else if mc.library_embeddings_paused(library_id).await {
        Some(EmbeddingPassState::Paused)
    } else {
        None
    }
}

impl LibraryPass {
    fn progress(&self, state: EmbeddingPassState) -> EmbeddingProgressMessage {
        EmbeddingProgressMessage {
            library: self.library.clone(),
            state,
            embedder: self.embedder.clone(),
            processed: self.processed,
            embedded: self.embedded,
            total: self.total,
        }
    }
}

#[async_trait]
impl RsSchedulerTask for EmbeddingTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        // A manual run and the recurring one must not embed the same medias twice.
        if !mc.try_start_embedding_backfill() {
            log_info(
                LogServiceType::Scheduler,
                "Media embedding already running, skipping".to_string(),
            );
            return Ok(());
        }
        let result = self.embed_libraries(&mc).await;
        mc.finish_embedding_backfill().await;
        result
    }
}

impl EmbeddingTask {
    async fn embed_libraries(&self, mc: &ModelController) -> RsResult<()> {
        let connected_user = &ConnectedUser::ServerAdmin;
        let libraries = mc.get_libraries(connected_user).await?;
        let libraries = if let Some(specific_library) = &self.specific_library {
            libraries
                .into_iter()
                .filter(|l| &l.id == specific_library)
                .collect()
        } else {
            libraries
        };

        for library in libraries {
            if must_stop(mc, &library.id).await.is_some() {
                continue;
            }
            // The server cannot read the files of these libraries.
            if mc.cache_get_library_crypt(&library.id).await {
                continue;
            }

            let total_count = match mc.count_medias_pending_embedding(&library.id).await {
                Ok(count) => count,
                Err(e) => {
                    log_error(
                        LogServiceType::Scheduler,
                        format!(
                            "Error getting embedding count for library {}: {:#}",
                            library.name, e
                        ),
                    );
                    0
                }
            };
            if total_count == 0 {
                continue;
            }

            log_info(
                LogServiceType::Scheduler,
                format!(
                    "Embedding {} medias for library {:?}",
                    total_count, library.name
                ),
            );
            let mut pass = LibraryPass {
                library: library.id.clone(),
                embedder: mc.embedder_spec().id.to_string(),
                total: total_count,
                processed: 0,
                embedded: 0,
                stopped: None,
            };
            mc.send_embedding_progress(pass.progress(EmbeddingPassState::Started));

            // Every way out of the pass is reported, so a listener that saw it start
            // also sees it end.
            let result = self.embed_library(mc, &library.name, &mut pass).await;
            let state = match &result {
                Err(_) => EmbeddingPassState::Failed,
                Ok(()) => pass.stopped.unwrap_or(EmbeddingPassState::Completed),
            };
            mc.send_embedding_progress(pass.progress(state));
            result?;
            log_info(
                LogServiceType::Scheduler,
                format!(
                    "Embedded {} of {} medias in library {}",
                    pass.embedded, pass.processed, library.name
                ),
            );
        }

        Ok(())
    }

    /// Embeds the pending medias of one library, counting them in `pass`.
    async fn embed_library(
        &self,
        mc: &ModelController,
        library_name: &str,
        pass: &mut LibraryPass,
    ) -> RsResult<()> {
        // Loading (and on first use downloading) the model is not a media failure.
        mc.prepare_embedder_for_images().await?;

        const CHUNK_SIZE: usize = 20;
        // That many failures in a row point at the model or the storage, not at
        // the medias: the pass leaves the library so they are not all given up on.
        const MAX_CONSECUTIVE_FAILURES: u32 = 5;
        let pass_started = embedding_pass_start();
        let mut consecutive_failures: u32 = 0;
        let mut previous_ids: Vec<String> = Vec::new();

        loop {
            // Before reading the next chunk: the library may be gone by now.
            if let Some(state) = must_stop(mc, &pass.library).await {
                pass.stopped = Some(state);
                return Ok(());
            }
            let media_ids = mc
                .get_medias_pending_embedding(&pass.library, pass_started, CHUNK_SIZE)
                .await?;
            // Medias uploaded or deleted since the pass started change what is left.
            // The total follows, so progress never runs past it.
            pass.total = pass.processed
                + mc.count_medias_left_in_embedding_pass(&pass.library, pass_started)
                    .await?;
            // The same chunk twice means nothing in it could be embedded or
            // recorded as failed (library unreadable): leave it to a later pass.
            if media_ids.is_empty() || media_ids == previous_ids {
                break;
            }

            for media_id in &media_ids {
                // Before each media, so a pause or turning embeddings off takes
                // effect as soon as the media in progress is done.
                if let Some(state) = must_stop(mc, &pass.library).await {
                    pass.stopped = Some(state);
                    return Ok(());
                }
                match mc.embed_media(&pass.library, media_id).await {
                    Ok(_) => {
                        pass.embedded += 1;
                        consecutive_failures = 0;
                    }
                    Err(e) => {
                        log_error(
                            LogServiceType::Scheduler,
                            format!("Error embedding media {}: {:#}", media_id, e),
                        );
                        consecutive_failures += 1;
                    }
                }
                pass.processed += 1;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    log_error(
                        LogServiceType::Scheduler,
                        format!(
                            "Stopping media embedding for library {}: {} failures in a row",
                            library_name, consecutive_failures
                        ),
                    );
                    pass.stopped = Some(EmbeddingPassState::Interrupted);
                    return Ok(());
                }
                mc.send_embedding_progress(pass.progress(EmbeddingPassState::Running));
            }
            previous_ids = media_ids;
        }

        Ok(())
    }
}
