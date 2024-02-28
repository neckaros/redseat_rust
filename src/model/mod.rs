pub mod store;
pub mod error;
pub mod users;
pub mod libraries;
pub mod server;
pub mod credentials;
pub mod backups;

pub mod tags;
pub mod people;
pub mod series;

use std::sync::Arc;



use crate::{domain::library::LibraryMessage, plugins::{sources::Source, PluginManager}};

use self::{store::SqliteStore, users::{ConnectedUser, UserRole}};
use error::{Result, Error};
use socketioxide::{extract::SocketRef, SocketIo};

#[derive(Clone)]
pub struct ModelController {
	store: Arc<SqliteStore>,
	io: Option<SocketIo>,
	plugin_manager: Arc<PluginManager>
}


// Constructor
impl ModelController {
	pub async fn new(store: SqliteStore, plugin_manager: PluginManager) -> Result<Self> {
		Ok(Self {
			store: Arc::new(store),
			io: None,
			plugin_manager: Arc::new(plugin_manager)
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

	pub async fn source_for_library(&self, library_id: &str) -> Result<Box<dyn Source>> {
		let library = self.store.get_library(library_id).await?.ok_or_else(|| Error::NotFound)?;
		let source = self.plugin_manager.source_for_library(library).await?;
		Ok(source)
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