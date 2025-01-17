use rs_plugin_common_interfaces::ElementType;
use serde::{Deserialize, Serialize};

use crate::{domain::{deleted::RsDeleted, episode::Episode, library::LibraryRole, media_progress::RsMediaProgress, media_rating::RsMediaRating}, error::RsResult, Error};

use super::{store::sql::SqlOrder, users::ConnectedUser, ModelController};



#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaProgressesQuery {
    pub after: Option<i64>,
    #[serde(default)]
    pub order: SqlOrder,
}


impl ModelController {
	pub async fn get_medias_progresses(&self, library_id: &str, query: MediaProgressesQuery, requesting_user: &ConnectedUser) -> RsResult<Vec<RsMediaProgress>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let deleted = store.get_medias_progresses(query).await?;

		Ok(deleted)
	}
}