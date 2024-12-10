use rs_plugin_common_interfaces::ElementType;
use serde::{Deserialize, Serialize};

use crate::{domain::{deleted::RsDeleted, episode::Episode, library::LibraryRole}, error::RsResult, Error};

use super::{store::sql::SqlOrder, users::ConnectedUser, ModelController};



#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeletedQuery {
    pub after: Option<i64>,
    #[serde(rename = "type")]
    pub kind: Option<ElementType>,
  
    #[serde(default)]
    pub order: SqlOrder,
}


impl ModelController {
	pub async fn get_deleted(&self, library_id: &str, query: DeletedQuery, requesting_user: &ConnectedUser) -> RsResult<Vec<RsDeleted>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let deleted = store.get_deleted(query).await?;

		Ok(deleted)
	}

    pub async fn add_deleted(&self, library_id: &str, deleted: RsDeleted, requesting_user: &ConnectedUser) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        store.add_deleted(deleted).await?;

		Ok(())
	}
}