use axum::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    domain::library::LibraryType,
    error::RsResult,
    model::{users::ConnectedUser, ModelController},
    tools::log::{log_error, log_info, LogServiceType},
};

use super::RsSchedulerTask;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IptvRefreshTask {
    pub specific_library: Option<String>,
}

#[async_trait]
impl RsSchedulerTask for IptvRefreshTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        let user = ConnectedUser::ServerAdmin;
        let libraries = mc.get_libraries(&user).await?;

        let iptv_libraries: Vec<_> = libraries
            .into_iter()
            .filter(|l| l.kind == LibraryType::Iptv)
            .filter(|l| {
                self.specific_library
                    .as_ref()
                    .map(|id| l.id == *id)
                    .unwrap_or(true)
            })
            .collect();

        for library in iptv_libraries {
            // The M3U URL is stored in library.root
            let has_m3u_url = library.root.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
            if !has_m3u_url {
                continue;
            }

            log_info(
                LogServiceType::Scheduler,
                format!("Refreshing IPTV library: {} ({})", library.name, library.id),
            );

            match mc.import_m3u(&library.id, None, &user).await {
                Ok(result) => {
                    log_info(
                        LogServiceType::Scheduler,
                        format!(
                            "IPTV refresh complete for {}: {} channels ({} new, {} updated, {} removed)",
                            library.name,
                            result.channels_added + result.channels_updated,
                            result.channels_added,
                            result.channels_updated,
                            result.channels_removed
                        ),
                    );
                }
                Err(e) => {
                    log_error(
                        LogServiceType::Scheduler,
                        format!("IPTV refresh failed for {}: {:#}", library.name, e),
                    );
                }
            }
        }

        Ok(())
    }
}
