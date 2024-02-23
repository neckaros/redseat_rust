use std::{cmp::Ordering, str::FromStr};

use rusqlite::{types::{FromSql, FromSqlError, FromSqlResult, ValueRef}, ToSql};
use serde::{Deserialize, Serialize};

use crate::domain::{library::{LibraryMessage, LibraryRole, LibraryType, ServerLibrary, ServerLibrarySettings}, ElementAction};

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
	pub name: Option<String>,
	pub source: Option<String>,
}
 
pub(super) fn map_library_for_user(library: ServerLibrary, user: &ConnectedUser) -> Option<ServerLibraryForRead> {
    match user {
        ConnectedUser::Server(user) => {
            let rights = user.libraries.iter().find(|x| x.id == library.id);
            if let Some(rights) = rights {
                let mut library = ServerLibraryForRead::from(library);
                if !rights.has_role(&LibraryRole::Admin) {
                    library.root = None;
                    library.settings = None;
                }
                Some(ServerLibraryForRead::from(library))
            } else {
                None
            }
        },
        ConnectedUser::Anonymous => None,
    }


}



#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct LibrarySocketMessage {
    pub action: ElementAction,
    pub library: ServerLibraryForRead
}

impl LibraryMessage {
    pub fn for_socket(&self, user: &ConnectedUser) -> Option<LibrarySocketMessage> {
        if let Some(library) =  map_library_for_user(self.library.clone(), user) {
            Some(LibrarySocketMessage { action: self.action.clone(), library })
        } else {
            None
        }
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