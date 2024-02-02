use std::str::FromStr;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use serde::{Deserialize, Serialize};

use super::error::{Error, Result};

// region:    --- User Role
#[serde(rename_all = "snake_case")] 
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum UserRole {
	Admin,
	Read,
	Write,
	None,
}
impl FromStr for UserRole {

    type Err = Error;

    fn from_str(input: &str) -> Result<UserRole> {
        match input {
            "admin"  => Ok(UserRole::Admin),
            "read"  => Ok(UserRole::Read),
            "write"  => Ok(UserRole::Write),
            _      => Ok(UserRole::None),
        }
    }
}
impl FromSql for UserRole {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            let r = UserRole::from_str(&as_string).map_err(|err| FromSqlError::InvalidType);
            r
        })
    }
}
// endregion: --- User Role


// region:    --- Preferences
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerUserPreferences {
    #[serde(default = "default_hidden_libraries")]
    pub hidden_libraries: Vec<String>,
}
fn default_hidden_libraries() -> Vec<String>{
    Vec::new()
}
// endregion:    --- Preferences

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerUser {
    pub id: String,
	pub name: String,
	pub role: UserRole,
	pub preferences: ServerUserPreferences,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerUserForUpdate {
	pub name: String,
	pub role: UserRole,
	pub preferences: ServerUserPreferences,
}
