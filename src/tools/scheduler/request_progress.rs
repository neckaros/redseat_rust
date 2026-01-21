use crate::{
    error::RsResult,
    model::{users::ConnectedUser, ModelController},
    tools::log::{log_error, log_info, LogServiceType},
};
use axum::async_trait;
use serde::{Deserialize, Serialize};

use super::RsSchedulerTask;

/// Task that periodically checks and updates progress of all active request processings
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequestProgressTask {}

impl RequestProgressTask {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl RsSchedulerTask for RequestProgressTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        log_info(
            LogServiceType::Scheduler,
            "Starting request progress check".to_string(),
        );

        let connected_user = &ConnectedUser::ServerAdmin;
        let libraries = mc.get_libraries(connected_user).await?;

        for library in libraries {
            match mc.process_active_requests(&library.id).await {
                Ok(processed_count) => {
                    if processed_count > 0 {
                        log_info(
                            LogServiceType::Scheduler,
                            format!(
                                "Processed {} active requests in library {}",
                                processed_count, library.name
                            ),
                        );
                    }
                }
                Err(e) => {
                    log_error(
                        LogServiceType::Scheduler,
                        format!(
                            "Error processing active requests for library {}: {:#}",
                            library.name, e
                        ),
                    );
                }
            }
        }

        log_info(
            LogServiceType::Scheduler,
            "Request progress check completed".to_string(),
        );

        Ok(())
    }
}
