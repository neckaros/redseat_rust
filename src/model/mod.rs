pub mod store;
pub mod error;
pub mod users;
pub mod libraries;
pub mod server;
pub mod credentials;
pub mod backups;
pub mod plugins;

pub mod tags;
pub mod people;
pub mod series;
pub mod episodes;
pub mod medias;
pub mod movies;

use std::{io::Read, path::PathBuf, pin::Pin, sync::Arc};
use strum::IntoEnumIterator;
use rs_plugin_url_interfaces::RsLink;
use crate::{domain::library::{LibraryMessage, LibraryRole}, plugins::{medias::{imdb::ImdbContext, tmdb::TmdbContext, trakt::TraktContext}, sources::{error::SourcesError, path_provider::PathProvider, AsyncReadPinBox, FileStreamResult, LocalSource, Source, SourceRead}, PluginManager}, server::get_server_file_path_array, tools::{image_tools::{resize_image_path, ImageSize, ImageSizeIter, ImageType}, log::log_info}};

use self::{store::SqliteStore, users::{ConnectedUser, UserRole}};
use error::{Result, Error};
use socketioxide::{extract::SocketRef, SocketIo};
use tokio::{fs::{self, remove_file, File}, io::{copy, AsyncRead, BufReader}};

#[derive(Clone)]
pub struct ModelController {
	store: Arc<SqliteStore>,
	io: Option<SocketIo>,
	pub plugin_manager: Arc<PluginManager>,
	pub trakt: TraktContext,
	pub tmdb: TmdbContext,
	pub imdb: ImdbContext
}


// Constructor
impl ModelController {
	pub async fn new(store: SqliteStore, plugin_manager: PluginManager) -> crate::Result<Self> {
		let tmdb = TmdbContext::new("4a01db3a73eed5cf17e9c7c27fd9d008".to_string()).await?;
		Ok(Self {
			store: Arc::new(store),
			io: None,
			plugin_manager: Arc::new(plugin_manager),
			trakt: TraktContext::new("455f81b3409a8dd140a941e9250ff22b2ed92d68003491c3976363fe752a9024".to_string()),
			tmdb,
			imdb: ImdbContext::new()
		})
	}
}




impl  ModelController {

	pub fn parse(&self, url: String) {
		self.plugin_manager.parse(url);
	}

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
		let source = self.plugin_manager.source_for_library(library, self.clone()).await?;
		Ok(source)
	}
	pub async fn library_source_for_library(&self, library_id: &str) -> Result<PathProvider> {
		let library = self.store.get_library(library_id).await?.ok_or_else(|| Error::NotFound)?;

		let path = if library.source == "virtual" {
			let path = get_server_file_path_array(vec!["libraries", &library.id]).await.map_err(|_| Error::FileNotFound("Unable to get virtual library local path".into()))?;
			path
		} else {
			if let Some(existing) = &library.root {
				let mut path = PathBuf::from(existing);
				path.push(".redseat");
				let new_path = path;
				new_path
			} else {
				get_server_file_path_array(vec!["libraries", &library.id]).await.map_err(|_| Error::FileNotFound("Unable to get virtual library local path".into()))?
			}
		};
		let source = PathProvider::new_for_local(path);
		Ok(source)
	}

	pub async fn library_image(&self, library_id: &str, folder: &str, id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> Result<FileStreamResult<AsyncReadPinBox>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;

        let m = self.library_source_for_library(&library_id).await?;
		let source_filepath = format!("{}/{}{}{}.webp", folder, id, ImageType::optional_to_filename_element(&kind), ImageSize::optional_to_filename_element(&size));
		let reader_response = m.get_file(&source_filepath, None).await;
		if let Some(int_size) = size {
			if let Err(error) = &reader_response {
				if matches!(error, SourcesError::NotFound(_)) {
					let original_filepath = format!("{}/{}{}.webp", folder, id, ImageType::optional_to_filename_element(&kind));
					let exist = m.exists(&original_filepath).await;
					if exist {
						log_info(crate::tools::log::LogServiceType::Other, format!("Creating image size: {} {} {} {}", folder, id, ImageType::optional_to_filename_element(&kind), int_size));
						resize_image_path(&m.get_gull_path(&original_filepath),  &m.get_gull_path(&source_filepath), int_size.to_size()).await?;
						let reader = m.get_file(&source_filepath, None).await?;
						if let SourceRead::Stream(reader) = reader {
							return Ok(reader);
						} else {
							return Err(Error::NotFound)
						}
					}
					
				}
			}
		}
		let reader = reader_response?;
		if let SourceRead::Stream(reader) = reader {
			return Ok(reader);
		} else {
			return Err(Error::NotFound)
		}
	}
	pub async fn has_library_image(&self, library_id: &str, folder: &str, id: &str, kind: Option<ImageType>, requesting_user: &ConnectedUser) -> Result<bool> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;

        let m = self.library_source_for_library(&library_id).await?;
		let source_filepath = format!("{}/{}{}.webp", folder, id, ImageType::optional_to_filename_element(&kind));
        let exist = m.exists(&source_filepath).await;
		Ok(exist)
	}
	pub async fn update_library_image<T: AsyncRead>(&self, library_id: &str, folder: &str, id: &str, kind: &Option<ImageType>, reader: T, requesting_user: &ConnectedUser) -> Result<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
		self.remove_library_image(library_id, folder, id, kind, requesting_user).await?;

        let m = self.library_source_for_library(&library_id).await?;

		let source_filepath = format!("{}/{}{}.webp", folder, id, ImageType::optional_to_filename_element(&kind));
		
		let (_, writer) = m.get_file_write_stream(&source_filepath).await?;
		tokio::pin!(reader);
		tokio::pin!(writer);
		copy(&mut reader, &mut writer).await?;

        Ok(())
	}

	pub async fn remove_library_image(&self, library_id: &str, folder: &str, id: &str, kind: &Option<ImageType>, requesting_user: &ConnectedUser) -> Result<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;

        let m = self.library_source_for_library(&library_id).await?;

		let source_filepath = format!("{}/{}{}.webp", folder, id, ImageType::optional_to_filename_element(&kind));
			let r = m.remove(&source_filepath).await;
			if r.is_ok() {
				log_info(crate::tools::log::LogServiceType::Other, format!("Deleted image {}", source_filepath));
			}

		for size in ImageSize::iter() {
			let source_filepath = format!("{}/{}{}{}.webp", folder, id, ImageType::optional_to_filename_element(&kind), size.to_filename_element());
			let r = m.remove(&source_filepath).await;
			if r.is_ok() {
				log_info(crate::tools::log::LogServiceType::Other, format!("Deleted image {}", source_filepath));
			}
		}
        Ok(())
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