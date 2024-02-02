use derive_more::From;
use serde::Serialize;
use serde_with::{serde_as, DisplayFromStr};


pub type Result<T> = core::result::Result<T, Error>;

#[serde_as]
#[derive(Debug, Serialize, From, strum_macros::AsRefStr)]
pub enum Error {
    CannotOpenDatabase,
	TxnCantCommitNoOpenTxn,
	CannotBeginTxnWithTxnFalse,
	CannotCommitTxnWithTxnFalse,

	// -- Externals
	#[from]
	Rusqlite(#[serde_as(as = "DisplayFromStr")] tokio_rusqlite::Error),
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