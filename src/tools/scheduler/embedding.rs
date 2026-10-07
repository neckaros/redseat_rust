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

            // Loading (and on first use downloading) the model is not a media failure.
            mc.get_embedder().await?;

            const CHUNK_SIZE: usize = 20;
            let pass_started = embedding_pass_start();
            let mut processed_count: u64 = 0;
            let mut embedded_count: u64 = 0;
            let mut previous_ids: Vec<String> = Vec::new();

            loop {
                // Stops between chunks when the library is turned off or removed.
                if !mc.library_embeddings_enabled(&library.id).await {
                    break;
                }
                let media_ids = mc
                    .get_medias_pending_embedding(&library.id, pass_started, CHUNK_SIZE)
                    .await?;
                // The same chunk twice means nothing in it could be embedded or
                // recorded as failed (library unreadable): leave it to a later pass.
                if media_ids.is_empty() || media_ids == previous_ids {
                    break;
                }

                for media_id in &media_ids {
                    match mc.embed_media(&library.id, media_id).await {
                        Ok(_) => {
                            embedded_count += 1;
                        }
                        Err(e) => {
                            log_error(
                                LogServiceType::Scheduler,
                                format!("Error embedding media {}: {:#}", media_id, e),
                            );
                        }
                    }
                    processed_count += 1;
                    let percent = ((processed_count * 100) / total_count).min(100);
                    mc.send_library_status(LibraryStatusMessage {
                        message: format!(
                            "Embedding media items... ({}/{}) - {}%",
                            processed_count, total_count, percent
                        ),
                        library: library.id.clone(),
                    });
                }
                previous_ids = media_ids;
            }

            mc.send_library_status(LibraryStatusMessage {
                message: format!(
                    "Completed media embedding for library {} ({}/{}) - 100%",
                    library.name, embedded_count, total_count
                ),
                library: library.id.clone(),
            });
            log_info(
                LogServiceType::Scheduler,
                format!(
                    "Embedded {} of {} medias in library {}",
                    embedded_count, processed_count, library.name
                ),
            );
        }

        Ok(())
    }
}
