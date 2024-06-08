use axum::{async_trait, Error};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::domain::movie::Movie;
use crate::domain::serie::{Serie, SerieStatus};
use crate::model::episodes::{EpisodeForUpdate, EpisodeQuery};
use crate::model::movies::MovieQuery;
use crate::model::series::SerieForUpdate;
use crate::server::update_ip;
use crate::tools::clock::{now, Clock};
use crate::{domain::library, error::RsResult, model::{series::SerieQuery, users::ConnectedUser, ModelController}, plugins::sources::Source, tools::{clock::UtcDate, log::{log_error, log_info}}};

use super::RsSchedulerTask;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RefreshIpTask {
}

impl RefreshIpTask {

}

#[async_trait]
impl RsSchedulerTask for RefreshIpTask {
    async fn execute(&self, _: ModelController) -> RsResult<()> {
        let connected_user = &ConnectedUser::ServerAdmin;
        log_info(crate::tools::log::LogServiceType::Scheduler, format!("Refresh IP"));
        let ips = update_ip().await;    
        log_info(crate::tools::log::LogServiceType::Scheduler, format!("Refreshed IP {:?}", ips));
        Ok(())
    }
}