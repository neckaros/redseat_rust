use derive_more::From;
use hyper::StatusCode;
use serde::Serialize;
use serde_with::{serde_as, DisplayFromStr};

use crate::{error::ClientError, model::medias::MediaSource, tools::image_tools::ImageError};

pub type SourcesResult<T> = core::result::Result<T, SourcesError>;

#[serde_as]
#[derive(Debug, Serialize, From, strum_macros::AsRefStr)]
pub enum SourcesError {

	NotImplemented,
    Error,
	Other(String),
	NotFound(Option<String>),

	
	UnableToFindLibrary(String, String),
	UnableToFindUser(String, String, String),
	UnableToFindUploadKey(String, String, String),
	UnableToFindPlugin(String, String),
	UnableToFindBackup(String, String),
	UnableToFindCredentials(String, String, String),
	UnableToFindMedia(String, String, String),
	UnableToFindSource(String, String, String, String),
	UnableToFindSerie(String, String, String),
	UnableToFindEpisodes(String, String),
	UnableToFindPerson(String, String, String),
	UnableToFindMovie(String, String, String),
	UnableToFindTag(String, String, String),
	
	#[from]
	Io(#[serde_as(as = "DisplayFromStr")] std::io::Error),

	#[from]
	Image(#[serde_as(as = "DisplayFromStr")] ImageError),

}

// region:    --- Error Boilerplate

impl core::fmt::Display for SourcesError {
	fn fmt(
		&self,
		fmt: &mut core::fmt::Formatter,
	) -> core::result::Result<(), core::fmt::Error> {
		write!(fmt, "{self:?}")
	}
}

impl std::error::Error for SourcesError {}

// endregion: --- Error Boilerplate

impl SourcesError {
	pub fn client_status_and_error(&self) -> (StatusCode, ClientError) {
		#[allow(unreachable_patterns)]
		match self {
			SourcesError::NotFound(_) => (StatusCode::NOT_FOUND, ClientError::NOT_FOUND),
			_ => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),
			
		}
	}
}