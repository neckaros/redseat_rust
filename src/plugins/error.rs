use derive_more::From;
use hyper::StatusCode;
use serde::Serialize;
use serde_with::serde_as;

use crate::error::ClientError;

pub type PluginsResult<T> = core::result::Result<T, PluginsError>;

#[serde_as]
#[derive(Debug, Serialize, From, strum_macros::AsRefStr)]
pub enum PluginsError {

    Error,


}

// region:    --- Error Boilerplate

impl core::fmt::Display for PluginsError {
	fn fmt(
		&self,
		fmt: &mut core::fmt::Formatter,
	) -> core::result::Result<(), core::fmt::Error> {
		write!(fmt, "{self:?}")
	}
}

impl std::error::Error for PluginsError {}

// endregion: --- Error Boilerplate

impl PluginsError {
	pub fn client_status_and_error(&self) -> (StatusCode, ClientError) {
		#[allow(unreachable_patterns)]
		match self {
			_ => (StatusCode::INTERNAL_SERVER_ERROR, ClientError::SERVICE_ERROR),
			
		}
	}
}