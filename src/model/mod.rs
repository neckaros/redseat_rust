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

use std::{path::PathBuf, sync::Arc};



use crate::{domain::library::{LibraryMessage, LibraryRole}, plugins::{sources::{error::SourcesError, path_provider::PathProvider, FileStreamResult, LocalSource, Source}, PluginManager}, server::get_server_file_path_array, tools::image_tools::{resize_image_path, ImageSize, ImageType}};

use self::{store::SqliteStore, users::{ConnectedUser, UserRole}};
use error::{Result, Error};
use socketioxide::{extract::SocketRef, SocketIo};
use tokio::{fs::File, io::BufReader};

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
	pub async fn library_source_for_library(&self, library_id: &str) -> Result<PathProvider> {
		let mut library = self.store.get_library(library_id).await?.ok_or_else(|| Error::NotFound)?;

		if library.source == "virtual" {
			let path = get_server_file_path_array(&mut vec!["libraries", &library.id]).await.map_err(|_| Error::FileNotFound("Unable to get virtual library local path".into()))?;
			library.root = Some(path.into_os_string().into_string().map_err(|_| Error::ServiceError("Unable to get virtual library local path".into(), None))?);
		} else {
			library.root = if let Some(existing) = &library.root {
				let mut path = PathBuf::from(existing);
				path.push(".redseat");
				let new_path = Some(path.into_os_string().into_string().map_err(|_| Error::ServiceError("Unable to get virtual library local path".into(), None))?);
				Ok(new_path)
			} else {
				Err(Error::ServiceError("Unable to get virtual library local path".into(), None))
			}?;
		}
		let source = PathProvider::new(library).await?;
		Ok(source)
	}

	pub async fn library_image(&self, library_id: &str, folder: &str, id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> Result<FileStreamResult<BufReader<File>>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;

        let m = self.library_source_for_library(&library_id).await?;
		let source_filepath = format!("{}/{}{}{}.webp", folder, id, ImageType::optional_to_filename_element(&kind), ImageSize::optional_to_filename_element(&size));
        let reader_response = m.get_file_read_stream(&source_filepath).await;
		if let Some(int_size) = size {
			if let Err(error) = &reader_response {
				if matches!(error, SourcesError::NotFound(_)) {
					let original_filepath = format!("{}/{}{}.webp", folder, id, ImageType::optional_to_filename_element(&kind));
					let exist = m.exists(&original_filepath).await;
					if exist {
						resize_image_path(&m.get_gull_path(&original_filepath),  &m.get_gull_path(&source_filepath), int_size.to_size()).await?;
						let reader_response = m.get_file_read_stream(&source_filepath).await?;
						return Ok(reader_response);
					}
					
				}
			}
		}

        Ok(reader_response?)
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