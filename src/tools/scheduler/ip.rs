use crate::domain::movie::Movie;
use crate::domain::serie::{Serie, SerieStatus};
use crate::model::episodes::{EpisodeForUpdate, EpisodeQuery};
use crate::model::movies::MovieQuery;
use crate::model::series::SerieForUpdate;
use crate::server::report_domain;
use crate::tools::clock::{now, Clock};
use crate::{
    domain::library,
    error::RsResult,
    model::{series::SerieQuery, users::ConnectedUser, ModelController},
    plugins::sources::Source,
    tools::{
        clock::UtcDate,
        log::{log_error, log_info},
    },
};
use axum::{async_trait, Error};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::RsSchedulerTask;

/// Reports the custom domain (or its absence) to the cloud.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReportDomainTask {}

impl ReportDomainTask {}

#[async_trait]
impl RsSchedulerTask for ReportDomainTask {
    async fn execute(&self, _: ModelController) -> RsResult<()> {
        crate::log_domain_report(report_domain().await);
        Ok(())
    }
}
