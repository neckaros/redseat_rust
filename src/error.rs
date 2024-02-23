use std::sync::Arc;

use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Serialize;
use derive_more::From;
use serde_json::json;

use nanoid::nanoid;

pub type Result<T> = core::result::Result<T, Error>;


#[derive(Debug, Serialize, From, strum_macros::AsRefStr)]
#[serde(tag = "type", content = "data")]
pub enum Error {
	Error { message: String},
	LoginFail,
	NotFound,

	// -- Auth errors.

	Forbiden,
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

	GenericRedseatError,
	
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
		let nanoid = nanoid!();
		println!("ERROR {}", self);
		
		println!("->> {:<12} - {:?}", "SERVICE_ERROR", self);
		let (status_code, client_error) = self.client_status_and_error();
	
		// -- If client error, build the new reponse.
		let error_json = json!({
						"error": {
							"type": client_error.as_ref(),
							"req_uuid": nanoid.to_string(),
						}
					});
	
		let mut error_response = (status_code, Json(error_json)).into_response();
		
		// Insert the Error into the reponse.
		error_response.extensions_mut().insert(Arc::new(self));

		error_response
	}
}

impl Error {
	pub fn client_status_and_error(&self) -> (StatusCode, ClientError) {
		#[allow(unreachable_patterns)]
		match self {
			Self::NotFound => (StatusCode::NOT_FOUND, ClientError::NOT_FOUND),

			Self::LoginFail => (StatusCode::FORBIDDEN, ClientError::LOGIN_FAIL),
			Self::Model(err) => err.client_status_and_error(),
			// -- Auth.
			Self::Forbiden => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Self::AuthFailNoAuthTokenCookie
			| Self::AuthFail
			| Self::AuthFailTokenWrongFormat => {
				(StatusCode::UNAUTHORIZED, ClientError::NO_AUTH)
			},
			Self::AuthFailExpiredToken => {
				(StatusCode::UNAUTHORIZED, ClientError::TOKEN_EXPIRED)
			}

			// -- Model.
			Self::TicketDeleteFailIdNotFound { .. } => {
				(StatusCode::BAD_REQUEST, ClientError::INVALID_PARAMS)
			}

			// -- Fallback.
			_ => (
				StatusCode::INTERNAL_SERVER_ERROR,
				ClientError::SERVICE_ERROR,
			),
		}
	}
}


#[derive(Debug, strum_macros::AsRefStr)]
#[allow(non_camel_case_types)]
pub enum ClientError {
	LOGIN_FAIL,
	NO_AUTH,
	TOKEN_EXPIRED,
	FORBIDDEN,
	NOT_FOUND,
	INVALID_PARAMS,
	SERVICE_ERROR,
}