use axum::async_trait;
use serde::{Deserialize, Serialize};
use crate::{error::RsResult, model::{ModelController, users::ConnectedUser}, tools::log::{log_error, log_info}};

use super::RsSchedulerTask;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FaceRecognitionTask {
    pub specific_library: Option<String>
}

impl FaceRecognitionTask {
    pub fn new() -> Self {
        Self {
            specific_library: None
        }
    }
}

#[async_trait]
impl RsSchedulerTask for FaceRecognitionTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        let connected_user = &ConnectedUser::ServerAdmin;
        let libraries = mc.get_libraries(&connected_user).await?;
        let libraries = if let Some(specific_library) = &self.specific_library {
            libraries.into_iter().filter(|l| &l.id == specific_library).collect()
        } else {
            libraries
        };

        for library in libraries {
            log_info(crate::tools::log::LogServiceType::Scheduler, format!("Processing face recognition for library {:?}", library.name));
            
            // Process unprocessed media in chunks
            const CHUNK_SIZE: usize = 50;
            let mut processed_count = 0;
            
            loop {
                let media_ids = mc.get_medias_for_face_processing(&library.id, CHUNK_SIZE).await?;
                
                if media_ids.is_empty() {
                    break;
                }
                
                for media_id in &media_ids {
                    match mc.process_media_faces(&library.id, media_id, &connected_user).await {
                        Ok(_) => {
                            processed_count += 1;
                        }
                        Err(e) => {
                            log_error(crate::tools::log::LogServiceType::Scheduler, format!("Error processing faces for media {}: {:#}", media_id, e));
                        }
                    }
                }
                
                // Run clustering periodically (after each chunk)
                match mc.cluster_unassigned_faces(&library.id).await {
                    Ok(result) => {
                        if result.clusters_created > 0 {
                            log_info(crate::tools::log::LogServiceType::Scheduler, format!("Created {} clusters from unassigned faces in library {}", result.clusters_created, library.name));
                        }
                    }
                    Err(e) => {
                        log_error(crate::tools::log::LogServiceType::Scheduler, format!("Error clustering faces for library {}: {:#}", library.name, e));
                    }
                }
                
                // If we got fewer than CHUNK_SIZE, we've processed all media
                if media_ids.len() < CHUNK_SIZE {
                    break;
                }
            }
            
            log_info(crate::tools::log::LogServiceType::Scheduler, format!("Processed {} media items for face recognition in library {}", processed_count, library.name));
        }
        
        log_info(crate::tools::log::LogServiceType::Scheduler, "Face recognition task completed".to_string());
        Ok(())
    }
}

