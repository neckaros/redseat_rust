use crate::{
    domain::library::LibraryStatusMessage,
    error::RsResult,
    model::{embeddings::embedding_pass_start, users::ConnectedUser, ModelController},
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

/// How a pass over one library ended.
struct LibraryPass {
    embedded: u64,
    processed: u64,
    /// Left early after too many failures in a row.
    interrupted: bool,
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
            if !mc.library_embeddings_enabled(&library.id).await {
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
            mc.send_library_status(LibraryStatusMessage {
                message: format!(
                    "Starting media embedding for library {} (0/{} - 0%)",
                    library.name, total_count
                ),
                library: library.id.clone(),
            });

            // Every way out of the pass is reported, so a listener that saw it start
            // also sees it end.
            let pass = self
                .embed_library(mc, &library.id, &library.name, total_count)
                .await;
            mc.send_library_status(LibraryStatusMessage {
                message: match &pass {
                    Ok(pass) if pass.interrupted => format!(
                        "Media embedding interrupted for library {} after repeated failures ({}/{})",
                        library.name, pass.embedded, total_count
                    ),
                    Ok(pass) => format!(
                        "Completed media embedding for library {} ({}/{}) - 100%",
                        library.name, pass.embedded, total_count
                    ),
                    Err(_) => format!("Media embedding failed for library {}", library.name),
                },
                library: library.id.clone(),
            });
            let pass = pass?;
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

    /// Embeds the pending medias of one library.
    async fn embed_library(
        &self,
        mc: &ModelController,
        library_id: &str,
        library_name: &str,
        total_count: u64,
    ) -> RsResult<LibraryPass> {
        // Loading (and on first use downloading) the model is not a media failure.
        mc.prepare_embedder_for_images().await?;

        const CHUNK_SIZE: usize = 20;
        // That many failures in a row point at the model or the storage, not at
        // the medias: the pass leaves the library so they are not all given up on.
        const MAX_CONSECUTIVE_FAILURES: u32 = 5;
        let pass_started = embedding_pass_start();
        let mut processed_count: u64 = 0;
        let mut embedded_count: u64 = 0;
        let mut consecutive_failures: u32 = 0;
        let mut interrupted = false;
        let mut previous_ids: Vec<String> = Vec::new();

        'library: loop {
            // Stops between chunks when the library is turned off or removed.
            if !mc.library_embeddings_enabled(library_id).await {
                break;
            }
            let media_ids = mc
                .get_medias_pending_embedding(library_id, pass_started, CHUNK_SIZE)
                .await?;
            // The same chunk twice means nothing in it could be embedded or
            // recorded as failed (library unreadable): leave it to a later pass.
            if media_ids.is_empty() || media_ids == previous_ids {
                break;
            }

            for media_id in &media_ids {
                match mc.embed_media(library_id, media_id).await {
                    Ok(_) => {
                        embedded_count += 1;
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
                processed_count += 1;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    log_error(
                        LogServiceType::Scheduler,
                        format!(
                            "Stopping media embedding for library {}: {} failures in a row",
                            library_name, consecutive_failures
                        ),
                    );
                    interrupted = true;
                    break 'library;
                }
                let percent = ((processed_count * 100) / total_count).min(100);
                mc.send_library_status(LibraryStatusMessage {
                    message: format!(
                        "Embedding media items... ({}/{}) - {}%",
                        processed_count, total_count, percent
                    ),
                    library: library_id.to_string(),
                });
            }
            previous_ids = media_ids;
        }

        Ok(LibraryPass {
            embedded: embedded_count,
            processed: processed_count,
            interrupted,
        })
    }
}
