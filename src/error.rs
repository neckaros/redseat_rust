use std::sync::Arc;

use axum::{http::StatusCode, response::{IntoResponse, Response}};
use serde::Serialize;
use derive_more::From;

pub type Result<T> = core::result::Result<T, Error>;


#[derive(Debug, Serialize, From, strum_macros::AsRefStr)]
#[serde(tag = "type", content = "data")]
pub enum Error {
	Error { message: String},
	LoginFail,

	// -- Auth errors.
	AuthFail,
	AuthFailNoAuthTokenCookie,
	AuthFailTokenWrongFormat,
	AuthFailInvalidToken,
	AuthFailExpiredToken,
	AuthFailNotForThisServer,

	// -- Model errors.
	TicketDeleteFailIdNotFound { id: u64 },
	
	// -- Database errors.
	UnableToOpenDatabase,
	StoreError,


    // -- Servers errors.
	ServerNoServerId,
	ServerMalformatedConfigFile,
	ServerUnableToAccessServerLocalFolder,
	ServerFileNotFound,


	
	// -- Externals

	#[from]
	Model(crate::model::error::Error),
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
impl IntoResponse for Error {
	fn into_response(self) -> Response {

		// Create a placeholder Axum reponse.
		let mut response = StatusCode::INTERNAL_SERVER_ERROR.into_response();

		// Insert the Error into the reponse.
		response.extensions_mut().insert(Arc::new(self));

		response
	}
}