use derive_more::From;
use hyper::StatusCode;
use serde::Serialize;
use serde_with::{serde_as, DisplayFromStr};

use crate::{domain::library::LibraryRole, error::ClientError, plugins::sources::error::SourcesError};

use super::{libraries::ServerLibraryForUpdate, users::{ConnectedUser, ServerUser, ServerUserForUpdate, UserRole}};


pub type Result<T> = core::result::Result<T, Error>;

#[serde_as]
#[derive(Debug, Serialize, From, strum_macros::AsRefStr)]
pub enum Error {

    UnableToParseEnum,

	NotFound,

    CannotOpenDatabase,
	TxnCantCommitNoOpenTxn,
	CannotBeginTxnWithTxnFalse,
	CannotCommitTxnWithTxnFalse,

	
	NotServerConnected,
	InsufficientUserRole {user: ConnectedUser, role: UserRole},
	InsufficientLibraryRole {user: ConnectedUser, library_id: String, role: LibraryRole},
	UserGetNotAuth { user: ConnectedUser, requested_user: String },
	UserListNotAuth { user: ConnectedUser},
	UserUpdateNotAuthorized { user: ServerUser, update_user: ServerUserForUpdate },
	UserRoleUpdateNotAuthOnlyAdmin,
	LibraryUpdateNotAuthorized { user: ServerUser, update_library: ServerLibraryForUpdate },

	// -- Externals
	#[from]
	TokioRusqlite(#[serde_as(as = "DisplayFromStr")] tokio_rusqlite::Error),
	#[from]
	Rusqlite(#[serde_as(as = "DisplayFromStr")] rusqlite::Error),
	#[from]
	Serde(#[serde_as(as = "DisplayFromStr")] serde_json::Error),
	#[from]
	Source(#[serde_as(as = "DisplayFromStr")] SourcesError),

}

// region:    --- Error Boilerplate

impl core::fmt::Display for Error {
	fn fmt(
		&self,
		fmt: &mut core::fmt::Formatter,
	) -> core::result::Result<(), core::fmt::Error> {
		write!(fmt, "{self:?}")
	}
}

impl std::error::Error for Error {}

// endregion: --- Error Boilerplate

impl Error {
	pub fn client_status_and_error(&self) -> (StatusCode, ClientError) {
		#[allow(unreachable_patterns)]
		match self {
			Error::NotFound => (StatusCode::NOT_FOUND, ClientError::NOT_FOUND),

			
			Error::UnableToParseEnum => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),

			Error::NotServerConnected => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Self::UserGetNotAuth { user: _, requested_user: _ } => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Error::CannotOpenDatabase => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),
			Error::TxnCantCommitNoOpenTxn => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),
			Error::CannotBeginTxnWithTxnFalse => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),
			Error::CannotCommitTxnWithTxnFalse => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),
			Error::InsufficientUserRole { user: _, role: _ } => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Error::InsufficientLibraryRole { user: _, library_id: _, role: _ } => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Error::UserGetNotAuth { user: _, requested_user: _ } => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Error::UserListNotAuth { user: _ } => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Error::UserUpdateNotAuthorized { user: _, update_user: _ } => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Error::UserRoleUpdateNotAuthOnlyAdmin => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Error::LibraryUpdateNotAuthorized { user: _, update_library: _ } => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Error::Rusqlite(_) | Error::TokioRusqlite(_) => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),
			Error::Serde(_) => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),
			Error::Source(_) => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),
			
		}
	}
}