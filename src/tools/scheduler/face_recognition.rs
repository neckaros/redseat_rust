use crate::{
    domain::library::LibraryStatusMessage,
    error::RsResult,
    model::{users::ConnectedUser, ModelController},
    tools::log::{log_error, log_info},
};
use axum::async_trait;
use serde::{Deserialize, Serialize};

use super::RsSchedulerTask;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FaceRecognitionTask {
    pub specific_library: Option<String>,
}

impl FaceRecognitionTask {
    pub fn new() -> Self {
        Self {
            specific_library: None,
        }
    }
}

#[async_trait]
impl RsSchedulerTask for FaceRecognitionTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        let connected_user = &ConnectedUser::ServerAdmin;
        let service = mc.get_face_recognition_service().await?;
        let libraries = mc.get_libraries(&connected_user).await?;
        let libraries = if let Some(specific_library) = &self.specific_library {
            libraries
                .into_iter()
                .filter(|l| &l.id == specific_library)
                .collect()
        } else {
            libraries
        };

        for library in libraries {
            log_info(
                crate::tools::log::LogServiceType::Scheduler,
                format!("Processing face recognition for library {:?}", library.name),
            );

            // Get total count of unprocessed medias before starting
            let total_count = match mc
                .count_medias_for_face_processing(&library.id, &connected_user)
                .await
            {
                Ok(count) => count,
                Err(e) => {
                    log_error(
                        crate::tools::log::LogServiceType::Scheduler,
                        format!(
                            "Error getting media count for library {}: {:#}",
                            library.name, e
                        ),
                    );
                    0
                }
            };
            println!("Total count: {}", total_count);
            // Send starting message
            if total_count > 0 {
                let message = format!(
                    "Starting face recognition for library {} (0/{} - 0%)",
                    library.name, total_count
                );
                println!("will try to send message: {:?}", message);
                mc.send_library_status(LibraryStatusMessage {
                    message,
                    library: library.id.clone(),
                });
            }

            // Match all unassigned faces to existing people (new people_face entries may enable matches)
            match mc
                .match_unassigned_faces_to_people(&library.id, &connected_user)
                .await
            {
                Ok(matched_count) => {
                    if matched_count > 0 {
                        log_info(
                            crate::tools::log::LogServiceType::Scheduler,
                            format!(
                                "Matched {} unassigned faces to existing people in library {}",
                                matched_count, library.name
                            ),
                        );
                    }
                }
                Err(e) => {
                    log_error(
                        crate::tools::log::LogServiceType::Scheduler,
                        format!(
                            "Error matching unassigned faces to people for library {}: {:#}",
                            library.name, e
                        ),
                    );
                }
            }

            // Process unprocessed media in chunks
            const CHUNK_SIZE: usize = 50;
            let mut processed_count = 0;

            loop {
                let media_ids = mc
                    .get_medias_for_face_processing(&library.id, CHUNK_SIZE)
                    .await?;

                if media_ids.is_empty() {
                    break;
                }

                for media_id in &media_ids {
                    match mc
                        .process_media_faces(
                            &library.id,
                            media_id,
                            &connected_user,
                            Some(service.clone()),
                        )
                        .await
                    {
                        Ok(_) => {
                            processed_count += 1;
                        }
                        Err(e) => {
                            log_error(
                                crate::tools::log::LogServiceType::Scheduler,
                                format!("Error processing faces for media {}: {:#}", media_id, e),
                            );
                        }
                    }
                    if total_count > 0 {
                        let percent = ((processed_count as u64 * 100) / total_count).min(100);
                        let message = format!(
                            "Processing media items... ({}/{}) - {}%",
                            processed_count, total_count, percent
                        );
                        mc.send_library_status(LibraryStatusMessage {
                            message,
                            library: library.id.clone(),
                        });
                    }
                }

                // Send progress update after each chunk
                if total_count > 0 {
                    let percent = ((processed_count as u64 * 100) / total_count).min(100);
                    let message = format!(
                        "Processing media items... ({}/{}) - {}%",
                        processed_count, total_count, percent
                    );
                    mc.send_library_status(LibraryStatusMessage {
                        message,
                        library: library.id.clone(),
                    });
                }

                // Run clustering periodically (after each chunk)
                match mc
                    .cluster_unassigned_faces(&library.id, &connected_user)
                    .await
                {
                    Ok(result) => {
                        if result.clusters_created > 0 {
                            log_info(
                                crate::tools::log::LogServiceType::Scheduler,
                                format!(
                                    "Created {} clusters from unassigned faces in library {}",
                                    result.clusters_created, library.name
                                ),
                            );
                        }
                    }
                    Err(e) => {
                        log_error(
                            crate::tools::log::LogServiceType::Scheduler,
                            format!(
                                "Error clustering faces for library {}: {:#}",
                                library.name, e
                            ),
                        );
                    }
                }

                // If we got fewer than CHUNK_SIZE, we've processed all media
                if media_ids.len() < CHUNK_SIZE {
                    break;
                }
            }

            // Final clustering pass to cluster any remaining unassigned faces that weren't clustered in chunks
            match mc
                .cluster_unassigned_faces(&library.id, &connected_user)
                .await
            {
                Ok(result) => {
                    if result.clusters_created > 0 {
                        log_info(crate::tools::log::LogServiceType::Scheduler, format!("Final clustering: Created {} clusters from remaining unassigned faces in library {}", result.clusters_created, library.name));
                    }
                }
                Err(e) => {
                    log_error(
                        crate::tools::log::LogServiceType::Scheduler,
                        format!(
                            "Error in final clustering for library {}: {:#}",
                            library.name, e
                        ),
                    );
                }
            }

            // Send completion message
            if total_count > 0 {
                let message = format!(
                    "Completed face recognition for library {} ({}/{}) - 100%",
                    library.name, processed_count, total_count
                );
                mc.send_library_status(LibraryStatusMessage {
                    message,
                    library: library.id.clone(),
                });
            }

            log_info(
                crate::tools::log::LogServiceType::Scheduler,
                format!(
                    "Processed {} media items for face recognition in library {}",
                    processed_count, library.name
                ),
            );
        }

        log_info(
            crate::tools::log::LogServiceType::Scheduler,
            "Face recognition task completed".to_string(),
        );

        // Send final completion message (without library ID since task is complete for all libraries)
        // Note: This is a general completion message, so we'll send it to all connected users
        // For simplicity, we'll skip this or send a generic message if needed

        Ok(())
    }
}
