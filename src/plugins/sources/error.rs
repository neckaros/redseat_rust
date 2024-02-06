use derive_more::From;
use hyper::StatusCode;
use serde::Serialize;
use serde_with::serde_as;

use crate::error::ClientError;

pub type SourcesResult<T> = core::result::Result<T, SourcesError>;

#[serde_as]
#[derive(Debug, Serialize, From, strum_macros::AsRefStr)]
pub enum SourcesError {

    Error,


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
			_ => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),
			
		}
	}
}