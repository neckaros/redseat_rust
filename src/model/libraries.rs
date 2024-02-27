use std::{cmp::Ordering, str::FromStr};

use nanoid::nanoid;
use serde::{Deserialize, Serialize};

use crate::domain::{library::{LibraryMessage, LibraryRole, LibraryType, ServerLibrary, ServerLibrarySettings}, ElementAction};

use super::{error::{Error, Result}, users::{ConnectedUser, UserRole}, ModelController};


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

impl ToString for LibraryType {
    fn to_string(&self) -> String {
        match &self {
            LibraryType::Photos => "photos",
            LibraryType::Shows => "shows",
            LibraryType::Movies => "movies",
            LibraryType::Iptv => "iptv",
        }.to_string()
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
pub struct ServerLibraryInvitation {
	pub code: String,
	pub expires: Option<String>,
	pub library: String,
	pub roles: Vec<LibraryRole>,
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
	pub root: Option<String>,
	pub settings: Option<ServerLibrarySettings>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerLibraryForAdd {
	pub name: String,
	pub source: String,
	pub root: Option<String>,
	pub settings: ServerLibrarySettings,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    pub crypt: Option<bool>,
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



impl ModelController {
    
	pub async fn get_library(&self, library_id: &str, requesting_user: &ConnectedUser) -> Result<Option<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Read)?;
		let lib = self.store.get_library(library_id).await?;
		if let Some(lib) = lib {
			let return_library = map_library_for_user(lib, &requesting_user);
			Ok(return_library)
		} else {
			Ok(None)
		}
	}

	pub async fn get_libraries(&self, requesting_user: &ConnectedUser) -> Result<Vec<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_role(&UserRole::Read)?;
		let libraries = self.store.get_libraries().await?.into_iter().flat_map(|l|  map_library_for_user(l, &requesting_user));
		Ok(libraries.collect::<Vec<super::libraries::ServerLibraryForRead>>())
	}
	pub async fn update_library(&self, library_id: &str, update: ServerLibraryForUpdate, requesting_user: &ConnectedUser) -> Result<Option<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
		self.store.update_library(library_id, update).await?;
        let library = self.store.get_library(library_id).await?;
        if let Some(library) = library { 
            self.send_library(LibraryMessage { action: crate::domain::ElementAction::Updated, library: library.clone() });
            Ok(map_library_for_user(library, &requesting_user))
        } else {
            Ok(None)
        }
	}

	pub async fn add_library(&self, library_for_add: ServerLibraryForAdd, requesting_user: &ConnectedUser) -> Result<Option<super::libraries::ServerLibraryForRead>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let library_id = nanoid!();
        let library = ServerLibrary {
                id: library_id.clone(),
                name: library_for_add.name,
                source: library_for_add.source,
                root: library_for_add.root,
                kind: library_for_add.kind,
                crypt: library_for_add.crypt,
                settings: library_for_add.settings,
            };
        self.store.add_library(library).await?;
        let user_id = requesting_user.user_id()?;
        self.store.add_library_rights(library_id.clone(), user_id, vec![LibraryRole::Admin]).await?;
        let library = self.store.get_library(&library_id).await?;
        if let Some(library) = library { 
            self.send_library(LibraryMessage { action: crate::domain::ElementAction::Added, library: library.clone() });
            Ok(Some(ServerLibraryForRead::from(library)))
        } else {
            Ok(None)
        }
	}
    
	pub async fn remove_library(&self, library_id: &str, requesting_user: &ConnectedUser) -> Result<ServerLibraryForRead> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
        let library = self.store.get_library(&library_id).await?;
        if let Some(library) = library { 
            self.store.remove_library(library_id.to_string()).await?;
            self.send_library(LibraryMessage { action: crate::domain::ElementAction::Removed, library: library.clone() });
            Ok(ServerLibraryForRead::from(library))
        } else {
            Err(Error::NotFound)
        }
	}

    pub async fn add_library_invitation(&self, library_id: &str, roles: Vec<LibraryRole>, requesting_user: &ConnectedUser) -> Result<super::libraries::ServerLibraryInvitation> {
        requesting_user.check_library_role(&library_id, LibraryRole::Admin)?;
        let invitation = ServerLibraryInvitation {
            code: nanoid!(),
            expires: None,
            library: library_id.to_string(),
            roles,
        };
        self.store.add_library_invitation(invitation.clone()).await?;
        Ok(invitation)
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