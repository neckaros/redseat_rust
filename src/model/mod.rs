mod store;
pub mod error;
pub mod users;
pub mod libraries;

use std::sync::Arc;



use self::{libraries::map_library_for_user, store::SqliteStore, users::{ConnectedUser, UserRole}};
use error::{Result, Error};

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
	pub async fn get_user_unchecked(&self, user_id: &str) -> Result<users::ServerUser> {
		self.store.get_user(&user_id).await
	}

	pub async fn get_user(&self, user_id: &str, requesting_user: &ConnectedUser) -> Result<users::ServerUser> {
		let id = user_id.to_string();
		if let ConnectedUser::Anonymous = requesting_user {
			return Err(Error::UserGetNotAuth { user: requesting_user.clone(), requested_user: id }) 
		} else if let ConnectedUser::Server(user) = &requesting_user {
			if user.id != id && user.role != UserRole::Admin {
				return Err(Error::UserGetNotAuth { user: requesting_user.clone(), requested_user: id })
			}
		}

		let user = self.store.get_user(&id).await;	

		user
	}

	pub async fn get_users(&self, requesting_user: &ConnectedUser) -> Result<Vec<users::ServerUser>> {
		if let ConnectedUser::Server(user) = &requesting_user {
			if user.role == UserRole::Admin {
				return self.store.get_users().await;	
			}
		}
		Err(Error::UserListNotAuth { user: requesting_user.clone() })
	}

	pub async fn get_libraries(&self, requesting_user: &ConnectedUser) -> Result<Vec<libraries::ServerLibraryForRead>> {
		let libraries = self.store.get_libraries().await?.into_iter().flat_map(|l|  map_library_for_user(l, &requesting_user));
		
		
		/*.map(|libs| {
			libs.into_iter().filter_map(|l| map_library_for_user(l, &requesting_user)).collect::<Vec<libraries::ServerLibraryForRead>>()
		});	*/
		Ok(libraries.collect::<Vec<libraries::ServerLibraryForRead>>())
	}
}