pub mod store;
pub mod error;
pub mod users;
pub mod libraries;
pub mod server;

use std::sync::Arc;



use crate::{domain::library::LibraryMessage, tools::log::log_info};

use self::{libraries::{map_library_for_user, ServerLibraryForRead, ServerLibraryForUpdate}, store::SqliteStore, users::{ConnectedUser, UserRole}};
use error::{Result, Error};
use serde_json::json;
use socketioxide::{extract::SocketRef, SocketIo};

#[derive(Clone)]
pub struct ModelController {
	store: Arc<SqliteStore>,
	io: Option<SocketIo>
}


// Constructor
impl ModelController {
	pub async fn new(store: SqliteStore) -> Result<Self> {
		Ok(Self {
			store: Arc::new(store),
			io: None,
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

	pub async fn get_library(&self, library_id: &str, requesting_user: &ConnectedUser) -> Result<Option<libraries::ServerLibraryForRead>> {
		let lib = self.store.get_library(library_id).await?;
		if let Some(lib) = lib {
			self.send_library(LibraryMessage { action: crate::domain::ElementAction::Added, library: lib.clone() });
			let return_library = map_library_for_user(lib, &requesting_user).map(|x| ServerLibraryForRead::from(x));
			Ok(return_library)
		} else {
			Ok(None)
		}
	}

	pub async fn get_libraries(&self, requesting_user: &ConnectedUser) -> Result<Vec<libraries::ServerLibraryForRead>> {
		let libraries = self.store.get_libraries().await?.into_iter().flat_map(|l|  map_library_for_user(l, &requesting_user));
		Ok(libraries.collect::<Vec<libraries::ServerLibraryForRead>>())
	}
	pub async fn update_library(&self, library_id: &str, update: ServerLibraryForUpdate, requesting_user: &ConnectedUser) -> Result<Option<libraries::ServerLibraryForRead>> {
		let lib = self.store.get_library(library_id).await?;
		if let Some(lib) = lib {
		let return_library = map_library_for_user(lib, &requesting_user).map(|x| ServerLibraryForRead::from(x));
			Ok(return_library)
		} else {
			Ok(None)
		}
	}
}



impl  ModelController {

	pub fn set_socket(&mut self, io: SocketIo) {
		self.io = Some(io);
	}

	fn for_connected_users<T: Clone>(&self, message: &T, action: fn(user: &ConnectedUser, socket: &SocketRef, message: T) -> ()) {
		let io = self.io.clone();
		if let Some(io) = io {
			if let Ok(sockets) = io.sockets() {
				for socket in sockets {
					if let Some(user) = socket.extensions.get::<ConnectedUser>() {
						action(&user, &socket, message.clone())
					}
				}
			}
		}
	}

	pub fn send_library(&self, message: LibraryMessage) {
		self.for_connected_users(&message, |user, socket, message| {
			if let Some(message) = message.for_socket(user) {
				let _ = socket.emit("library", message);
			}
		});
	}
	


}