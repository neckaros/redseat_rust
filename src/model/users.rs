use std::{cmp::Ordering, str::FromStr};

use rusqlite::{
    types::{FromSql, FromSqlError, FromSqlResult, ValueRef},
    ToSql,
};
use serde::{Deserialize, Serialize};
use strum_macros::EnumString;

use crate::{domain::library::{LibraryRole, LibraryType}, tools::auth::{ClaimsLocal, ClaimsLocalType}};

use super::{error::{Error, Result}, libraries::ServerLibraryForRead};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum ConnectedUser {
    Server(ServerUser),
    Share(ClaimsLocal),
    Anonymous,
    ServerAdmin
}

impl ConnectedUser {
    pub fn is_registered(&self) -> bool {
        matches!(&self, ConnectedUser::Server(_)) 
    }
    pub fn is_admin(&self) -> bool {
        if self == &ConnectedUser::ServerAdmin {
            true
        } else if let ConnectedUser::Server(user) = &self {
            user.is_admin()
        } else if let ConnectedUser::Share(claims) = &self {
            if claims.kind == ClaimsLocalType::Admin {
                true
            } else if let ClaimsLocalType::UserRole(role) = &claims.kind {
                role == &UserRole::Admin
            } else {
                false
            }
        } else {
            false
        }
    }
    pub fn user_id(&self) -> Result<String> {
        if let ConnectedUser::Server(user) = &self {
            Ok(user.id.clone())
        } else {
            Err(Error::NotServerConnected)
        }
    }
    pub fn check_role(&self, role: &UserRole) -> Result<()> {
        if self.is_admin() {
            Ok(())
        } else if let ConnectedUser::Share(claims) = &self {
            match &claims.kind {
                ClaimsLocalType::File(_, _) => {
                    Ok(()) 
                },
                ClaimsLocalType::UserRole(_) => Err(Error::ShareTokenInsufficient),
                ClaimsLocalType::Admin => Ok(()),
            }
        } else if let ConnectedUser::Server(user) = &self {
            if user.has_role(role) {
                Ok(())
            } else {
                Err(Error::InsufficientUserRole { user: self.clone(), role: role.clone() })
            }
        } else {
            Err(Error::InsufficientUserRole { user: self.clone(), role: role.clone() })
        }
    }
    pub fn check_library_role(&self, library_id: &str, role: LibraryRole) -> Result<()> {
        if self.is_admin() {
            Ok(())
        } else if let ConnectedUser::Server(user) = &self {
            if user.has_library_role(&library_id, &role) {
                Ok(())
            } else {
                Err(Error::InsufficientLibraryRole { user: self.clone(), library_id: library_id.to_string(), role: role.clone() })
            }
        } else if let ConnectedUser::Share(claims) = &self {
            match &claims.kind {
                ClaimsLocalType::File(library, _) => {
                    if library == library_id { 
                        Ok(()) 
                    } else {
                        Err(Error::ShareTokenInsufficient)
                    }
                },
                ClaimsLocalType::UserRole(_) => Err(Error::ShareTokenInsufficient),
                ClaimsLocalType::Admin => Err(Error::ShareTokenInsufficient),
            }
        } else {
            Err(Error::NotServerConnected)
        }
    }
    
    pub fn check_file_role(&self, library_id: &str, file_id: &str, role: LibraryRole) -> Result<()> {
        if self.is_admin() {
            Ok(())
        } else if let ConnectedUser::Server(user) = &self {
            if user.has_library_role(&library_id, &role) {
                Ok(())
            } else {
                Err(Error::InsufficientLibraryRole { user: self.clone(), library_id: library_id.to_string(), role: role.clone() })
            }
        } else if let ConnectedUser::Share(claims) = &self {
            match &claims.kind {
                ClaimsLocalType::File(_, id) => {
                    if id == file_id { 
                        Ok(()) 
                    } else {
                        Err(Error::ShareTokenInsufficient)
                    }
                },
                ClaimsLocalType::UserRole(_) => Err(Error::ShareTokenInsufficient),
                ClaimsLocalType::Admin => Err(Error::ShareTokenInsufficient),
            }
        } else {
            Err(Error::NotServerConnected)
        }
    }
}

// region:    --- User Role
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, EnumString, Default)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum UserRole {
    Admin,
    Read,
    Write,
    #[default]
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
/*
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
}*/
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
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerUserPreferences {
    #[serde(default = "default_hidden_libraries")]
    pub hidden_libraries: Vec<String>,
}
fn default_hidden_libraries() -> Vec<String> {
    Vec::new()
}
// endregion:    --- Preferences


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ServerUserLibrariesRights {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    pub roles: Vec<LibraryRole>,
}

impl ServerUserLibrariesRights {
    pub fn has_role(&self, role: &LibraryRole) -> bool {
        self.roles.iter().any(|r| r >= role)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerUserLibrariesRightsWithUser {
    pub id: String,
    pub user_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    pub roles: Vec<LibraryRole>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerLibrariesRightsForAdd {
    pub library_id: String,
    pub user_id: String,
    pub roles: Vec<LibraryRole>,
}



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ServerUser {
    pub id: String,
    pub name: String,
    pub role: UserRole,
    pub preferences: ServerUserPreferences,
    pub libraries: Vec<ServerUserLibrariesRights>
}

impl ServerUser {
    pub fn is_admin(&self) -> bool {
        matches!(&self.role, UserRole::Admin)
    }
    pub fn has_role(&self, role: &UserRole) -> bool {
        &self.role >= role
    }
    pub fn has_library_role(&self, library_id: &str, role: &LibraryRole) -> bool {
        let libraries = &self.libraries.clone();
        let found = libraries.into_iter().find(|l| l.id == library_id);
        if let Some(found) = found {
            found.has_role(role)
        } else {
            false
        }
    }
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