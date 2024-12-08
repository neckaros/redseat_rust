use std::{sync::{Arc, PoisonError}, time::SystemTimeError};

use axum::{extract::multipart, http::StatusCode, response::{IntoResponse, Response}, Json};
use ndarray::ShapeError;
use rs_plugin_common_interfaces::CredentialType;
use serde::{Deserialize, Serialize};
use derive_more::From;
use serde_json::json;
use serde_with::{serde_as, DisplayFromStr};
use nanoid::nanoid;
use crate::{domain::{MediaElement, MediasIds}, plugins::sources::error::SourcesError, tools::{image_tools, log::{log_error, LogServiceType}}};


pub type Result<T> = core::result::Result<T, Error>;
pub type RsResult<T> = Result<T>;
pub type RsError = Error;

#[serde_as]
#[derive(Debug, Serialize, From, strum_macros::AsRefStr)]
#[serde(tag = "type", content = "data")]
pub enum Error {
	Error(String),
	Message(String),
	LoginFail,
	NotFound,
	
	NotImplemented(String),

	UnavailableForCryptedLibraries,
	CryptError(String),
	TimeCreationError,
	TraktTooManyUpdates,

	// Server Error
	InvalidPublicKey,
	InvalidPrivateKey,

	// Range Error 
	InvalidRangeHeader,
	NoRangeHeader,

	NotAMediaId(String),
	NoMediaIdRequired(Box<MediasIds>),

	// Prediction Error 

	NoModelFound,
	ModelMappingNotFound,
	ModelNotFound(String),


	// Plugins Error 
	PluginNotFound(String),
	PluginUnsupportedCall(String, String),
	PluginUnsupportedCredentialType(CredentialType, Option<CredentialType>),
	PluginError(i32, String),

	// -- Auth errors.

	Forbiden,
	AuthFail,
	ServerNotYetRegistered,
	ServerAlreadyRegistered,
	ServerAlreadyOwned,
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
	Model(#[serde_as(as = "DisplayFromStr")] crate::model::error::Error),

	#[from]
	Io(#[serde_as(as = "DisplayFromStr")] std::io::Error),

	#[from]
	Source(#[serde_as(as = "DisplayFromStr")] SourcesError),

	#[from]
	Serde(#[serde_as(as = "DisplayFromStr")] serde_json::Error),

	
	#[from]
	ORT(#[serde_as(as = "DisplayFromStr")] ort::Error),

	#[from]
	Image(#[serde_as(as = "DisplayFromStr")] image::ImageError),

	
	#[from]
	RsImage(#[serde_as(as = "DisplayFromStr")] image_tools::ImageError),


	#[from]
	Multipart(#[serde_as(as = "DisplayFromStr")] multipart::MultipartError),
	#[from]
	Reqwest(#[serde_as(as = "DisplayFromStr")] reqwest::Error),

	
	#[from]
	NdShapeError(#[serde_as(as = "DisplayFromStr")] ShapeError),


	#[from]
	YtDl(#[serde_as(as = "DisplayFromStr")] youtube_dl::Error),
	
	
	#[from]
	SystemnTime(#[serde_as(as = "DisplayFromStr")] SystemTimeError),

	
	#[from]
	Extism(#[serde_as(as = "DisplayFromStr")] extism::Error),

	
	#[from]
	Zip(#[serde_as(as = "DisplayFromStr")] zip::result::ZipError),
	
	#[from]
	Trash(#[serde_as(as = "DisplayFromStr")] trash::Error),

	#[from]
	PadError(#[serde_as(as = "DisplayFromStr")] cbc::cipher::inout::PadError),

	
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
		log_error(LogServiceType::Other, format!("{:?}", self));
		let (status_code, client_error) = self.client_status_and_error();
	
		// -- If client error, build the new reponse.
		let error_json = json!({
						"error": {
							"value": client_error,
							"reqUuid": nanoid.to_string(),
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

			Self::UnavailableForCryptedLibraries => (StatusCode::UNPROCESSABLE_ENTITY, ClientError::NOT_FOR_CRYPTED_LIBRARY),
			

			Self::LoginFail => (StatusCode::FORBIDDEN, ClientError::LOGIN_FAIL),
			Self::Model(err) => err.client_status_and_error(),
			// -- Auth.
			Self::ServerAlreadyRegistered => (StatusCode::FORBIDDEN, ClientError::SERVER_ALREADY_REGISTERED),
			Self::ServerAlreadyOwned => (StatusCode::FORBIDDEN, ClientError::SERVER_ALREADY_OWNED),
			Self::Forbiden => (StatusCode::FORBIDDEN, ClientError::FORBIDDEN),
			Self::AuthFailNoAuthTokenCookie
			| Self::AuthFail
			| Self::AuthFailTokenWrongFormat
			| Self::AuthFailInvalidToken
			| Self::AuthFailNotForThisServer => {
				(StatusCode::UNAUTHORIZED, ClientError::NO_AUTH)
			},
			Self::AuthFailExpiredToken => {
				(StatusCode::UNAUTHORIZED, ClientError::TOKEN_EXPIRED)
			}

			// -- Model.
			Self::TicketDeleteFailIdNotFound { .. } => {
				(StatusCode::BAD_REQUEST, ClientError::INVALID_PARAMS)
			},
			// -- Plugin 
			Self::PluginError(code, error) => {
				(StatusCode::INTERNAL_SERVER_ERROR, ClientError::PLUGIN{ code: code.clone(), message: error.to_string()})
			},
			// -- Prediction
			Self::NoModelFound => (StatusCode::NOT_FOUND, ClientError::NOT_FOUND),
			// -- Fallback.
			_ => (
				StatusCode::INTERNAL_SERVER_ERROR,
				ClientError::SERVICE_ERROR,
			),
		}
	}
}


#[derive(Debug, Serialize, Deserialize, strum_macros::AsRefStr)]
#[serde(tag = "type")]
#[allow(non_camel_case_types)]
pub enum ClientError {
	LOGIN_FAIL,
	NO_AUTH,
	TOKEN_EXPIRED,
	FORBIDDEN,
	SERVER_ALREADY_REGISTERED,
	SERVER_ALREADY_OWNED,
	NOT_FOUND,
	NOT_FOR_CRYPTED_LIBRARY,
	INVALID_PARAMS,
	SERVICE_ERROR,
	Custom(String),
	DUPLICATE(DuplicateClientError),
	PLUGIN {code: i32, message: String}
}


#[derive(Debug, Serialize, Deserialize)]
pub struct DuplicateClientError {
	pub id: String,
	pub element: MediaElement
}

impl Error {
	pub fn from_code(code: i32) -> Error {
		if code != 404 {
			Error::NotFound
		} else if code != 401 {
			Error::AuthFailExpiredToken
		} else if code != 403 {
			Error::Forbiden
		} else {
			Error::Error(format!("Plugin error: {}", code))
		}
	}
}