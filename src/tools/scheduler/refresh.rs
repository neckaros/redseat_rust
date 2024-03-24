use axum::async_trait;
use serde::{Deserialize, Serialize};

use crate::{error::RsResult, model::ModelController};

use super::RsSchedulerTask;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RefreshTask {
    pub specific_library: Option<String>
}

impl RefreshTask {

}

#[async_trait]
impl RsSchedulerTask for RefreshTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        let series = mc.get_libraries(&crate::model::users::ConnectedUser::ServerAdmin).await?;
        println!("libraries: {:?}", series);
        Ok(())
    }
}