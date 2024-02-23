use std::{cmp::Ordering, str::FromStr};

use rusqlite::{types::{FromSql, FromSqlError, FromSqlResult, ValueRef}, ToSql};
use serde::{Deserialize, Serialize};

use crate::domain::library::{LibraryRole, LibraryType, ServerLibrary, ServerLibrarySettings};

use super::{error::{Error, Result}, users::ConnectedUser};


// region:    --- Library type

impl FromStr for LibraryType {
    type Err = Error;
    fn from_str(input: &str) -> Result<LibraryType> {
        match input {
            "photos"  => Ok(LibraryType::Photos),
            "shows"  => Ok(LibraryType::Shows),
            "movies"  => Ok(LibraryType::Movies),
            "iptv"     => Ok(LibraryType::Iptv),
            _      => Err(Error::UnableToParseEnum),
        }
    }
}

impl FromSql for LibraryType {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            let r = LibraryType::from_str(&as_string).map_err(|_| FromSqlError::InvalidType);
            r
        })
    }
}

// endregion:    --- 

// region:    --- Library Role



impl From<u8> for &LibraryRole {
    fn from(level: u8) -> Self {
        if level < 9 {
            return &LibraryRole::None;
        } else if level < 20 {
            return &LibraryRole::Read;
        } else if level < 30 {
            return &LibraryRole::Write;
        } else if level == 254 {
            return &LibraryRole::Admin;
        }
        return &LibraryRole::None;
    }
}
impl From<&LibraryRole> for u8 {
    fn from(role: &LibraryRole) -> Self {
        match role {
            &LibraryRole::Admin => 254,
            &LibraryRole::Write => 20,
            &LibraryRole::Read => 10,
            &LibraryRole::None => 0,
        }
    }
}

impl PartialOrd for LibraryRole {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let a = u8::from(self);
        let b = u8::from(other);
        Some(a.cmp(&b))
    }
}

impl FromStr for LibraryRole {
    type Err = Error;
    fn from_str(input: &str) -> Result<LibraryRole> {
        match input {
            "admin"  => Ok(LibraryRole::Admin),
            "read"  => Ok(LibraryRole::Read),
            "write"  => Ok(LibraryRole::Write),
            _      => Ok(LibraryRole::None),
        }
    }
}

impl core::fmt::Display for LibraryRole {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            LibraryRole::Admin  => write!(f, "admin"),
            LibraryRole::Read  => write!(f, "read"),
            LibraryRole::Write  => write!(f, "write"),
            LibraryRole::None  => write!(f, "none"),
        }
    }
}

impl FromSql for LibraryRole {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            let r = LibraryRole::from_str(&as_string).map_err(|_| FromSqlError::InvalidType);
            r
        })
    }
}

impl ToSql for LibraryRole {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            LibraryRole::Admin  => "admin".to_sql(),
            LibraryRole::Read  => "read".to_sql(),
            LibraryRole::Write  => "write".to_sql(),
            LibraryRole::None  => "none".to_sql(),
        }
    }
}

// endregion: ---

// region:    --- Library Settings

impl FromSql for ServerLibrarySettings {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {

            let r = serde_json::from_str::<ServerLibrarySettings>(&as_string).map_err(|_| FromSqlError::InvalidType)?;

            Ok(r)
        })
    }
}
// endregion:    --- 



#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerLibraryForRead {
    pub id: String,
	pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
	pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crypt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<ServerLibrarySettings>
}
impl From<ServerLibrary> for ServerLibraryForRead {
    fn from(lib: ServerLibrary) -> Self {
        ServerLibraryForRead {
            id: lib.id,
            name: lib.name,
            source: Some(lib.source),
            root: lib.root,
            kind: lib.kind,
            crypt: lib.crypt,
            settings:Some(lib.settings),
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerLibraryForUpdate {
    pub id: String,
	pub name: Option<String>,
	pub source: Option<String>,
}
 
pub(super) fn map_library_for_user(library: ServerLibrary, user: &ConnectedUser) -> Option<ServerLibraryForRead> {
    match user {
        ConnectedUser::Server(user) => {
            //println!("GO");
            let rights = user.libraries.iter().find(|x| x.id == library.id);
            //println!("GA {:?}", user.libraries);
            if let Some(rights) = rights {
                let mut library = ServerLibraryForRead::from(library);
                if !rights.has_role(&LibraryRole::Admin) {
                    library.root = None;
                    library.settings = None;
                }
                return Some(ServerLibraryForRead::from(library))
            } else {
                return None
            }
        },
        ConnectedUser::Anonymous => None,
    }


}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role() {
        assert_eq!(LibraryRole::Read < LibraryRole::Write, true);
        assert_eq!(LibraryRole::Write < LibraryRole::Admin, true);
        assert_eq!(LibraryRole::None < LibraryRole::Read, true);
        assert_eq!(LibraryRole::Admin > LibraryRole::Write, true);
        assert_eq!(LibraryRole::Write > LibraryRole::Read, true);
        assert_eq!(LibraryRole::Read > LibraryRole::None, true);

        assert_eq!(LibraryRole::Read > LibraryRole::Write, false);

    }
}