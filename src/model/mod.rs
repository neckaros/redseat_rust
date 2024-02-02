mod store;
pub mod error;
pub mod users;

use std::sync::Arc;



use self::store::SqliteStore;
use error::Result;

#[derive(Clone)]
pub struct ModelController {
	store: Arc<SqliteStore>
}


// Constructor
impl ModelController {
	pub async fn new() -> Result<Self> {
		Ok(Self {
			store: Arc::new(SqliteStore::new().await.unwrap())
		})
	}
}




impl  ModelController {
	pub async fn get_user(&self, user_id: &str) -> Result<users::ServerUser> {
		let id = user_id.to_string();
		let user = self.store.get_user(&id).await;	
		user
	}

	pub async fn get_users(&self) -> Result<Vec<users::ServerUser>> {

		let users = self.store.get_users().await;	
		users
	}
}