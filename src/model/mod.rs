mod store;
pub mod error;

use std::sync::Arc;
use serde::{Deserialize, Serialize};



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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerUser {
    user_id: String,
	name: String
}
impl  ServerUser {
	pub fn user_id(&self) -> &String {
		&self.user_id
	}
}

impl  ModelController {
	pub async fn get_user(&self, user_id: &str) -> Result<ServerUser> {
		let id = user_id.to_string();
		let user = self.store.get_user(&id).await;	
		user
	}
}