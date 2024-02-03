use std::{cmp::Ordering, str::FromStr};

use rusqlite::{
    types::{FromSql, FromSqlError, FromSqlResult, ValueRef},
    ToSql,
};
use serde::{Deserialize, Serialize};

use super::{error::{Error, Result}, libraries::{LibraryRole, LibraryType}};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ConnectedUser {
    Server(ServerUser),
    Anonymous
}

// region:    --- User Role
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Admin,
    Read,
    Write,
    None,
}
impl From<u8> for &UserRole {
    fn from(level: u8) -> Self {
        if level < 9 {
            return &UserRole::None;
        } else if level < 20 {
            return &UserRole::Read;
        } else if level < 30 {
            return &UserRole::Write;
        } else if level == 254 {
            return &UserRole::Admin;
        }
        return &UserRole::None;
    }
}
impl From<&UserRole> for u8 {
    fn from(role: &UserRole) -> Self {
        match role {
            &UserRole::Admin => 254,
            &UserRole::Write => 20,
            &UserRole::Read => 10,
            &UserRole::None => 0,
        }
    }
}

impl PartialOrd for UserRole {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let a = u8::from(self);
        let b = u8::from(other);
        Some(a.cmp(&b))
    }
}

impl FromStr for UserRole {
    type Err = Error;

    fn from_str(input: &str) -> Result<UserRole> {
        match input {
            "admin" => Ok(UserRole::Admin),
            "read" => Ok(UserRole::Read),
            "write" => Ok(UserRole::Write),
            _ => Ok(UserRole::None),
        }
    }
}
impl FromSql for UserRole {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            let r = UserRole::from_str(&as_string).map_err(|_| FromSqlError::InvalidType);
            r
        })
    }
}

impl ToSql for UserRole {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            UserRole::Admin => "admin".to_sql(),
            UserRole::Read => "read".to_sql(),
            UserRole::Write => "write".to_sql(),
            UserRole::None => "none".to_sql(),
        }
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
fn default_hidden_libraries() -> Vec<String> {
    Vec::new()
}
// endregion:    --- Preferences


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerUserLibrariesRights {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    pub roles: Vec<LibraryRole>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerUserLibrariesRightsWithUser {
    pub id: String,
    pub user_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    pub roles: Vec<LibraryRole>,
}



#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerUser {
    pub id: String,
    pub name: String,
    pub role: UserRole,
    pub preferences: ServerUserPreferences,
    pub libraries: Vec<ServerUserLibrariesRights>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerUserForUpdate {
    pub id: String,
    pub name: Option<String>,
    pub role: Option<UserRole>,
    pub preferences: Option<ServerUserPreferences>,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role() {
        assert_eq!(UserRole::Read < UserRole::Write, true);
        assert_eq!(UserRole::Write < UserRole::Admin, true);
        assert_eq!(UserRole::None < UserRole::Read, true);
        assert_eq!(UserRole::Admin > UserRole::Write, true);
        assert_eq!(UserRole::Write > UserRole::Read, true);
        assert_eq!(UserRole::Read > UserRole::None, true);

    }
}