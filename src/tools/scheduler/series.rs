use axum::async_trait;

use crate::{error::RsResult, model::ModelController};

use super::RsSchedulerTask;

pub struct SerieTask {

}

impl SerieTask {
    pub fn new(params: String) -> RsResult<Self> {
        Ok(Self {})
    }
}

#[async_trait]
impl RsSchedulerTask for SerieTask {
    async fn execute(&self, mc: ModelController) -> RsResult<()> {
        let series = mc.get_libraries(&crate::model::users::ConnectedUser::ServerAdmin).await?;
        println!("libraries: {:?}", series);
        Ok(())
    }
}