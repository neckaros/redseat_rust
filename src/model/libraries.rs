use std::{cmp::Ordering, str::FromStr};

use nanoid::nanoid;
use rs_plugin_common_interfaces::RsRequest;
use serde::{Deserialize, Serialize};
use tokio::fs::read_dir;

use crate::{domain::{library::{LibraryMessage, LibraryRole, LibraryType, ServerLibrary, ServerLibrarySettings}, ElementAction}, plugins::sources::{Source, SourceRead}, tools::auth::{sign_local, ClaimsLocal, ClaimsLocalType}};

use super::{error::{Error, Result}, users::{ConnectedUser, UserRole}, ModelController};




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


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
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
    pub settings: Option<ServerLibrarySettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<LibraryRole>>,

    
    #[serde(default)]
    pub hidden: bool,
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
            roles: None,

            ..Default::default()
        }
    }
}
impl ServerLibraryForRead {
    fn into_with_role(lib: ServerLibrary, roles: &Vec<LibraryRole>) -> Self {
        ServerLibraryForRead {
            id: lib.id,
            name: lib.name,
            source: Some(lib.source),
            root: lib.root,
            kind: lib.kind,
            crypt: lib.crypt,
            settings:Some(lib.settings),
            roles: Some(roles.to_owned()),
            ..Default::default()
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerLibraryForUpdate {
	pub name: Option<String>,
	pub source: Option<String>,
	pub root: Option<String>,
	pub settings: Option<ServerLibrarySettings>,
	pub credentials: Option<String>,
    pub plugin: Option<String>,
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
	pub credentials: Option<String>,
    pub plugin: Option<String>,
}
 
pub(super) fn map_library_for_user(library: ServerLibrary, user: &ConnectedUser) -> Option<ServerLibraryForRead> {
    
    let hidden = user.has_hidden_library(library.id.clone());
    match user {
        ConnectedUser::Server(user) => {
            let rights = user.libraries.iter().find(|x| x.id == library.id);
            if let Some(rights) = rights {
                let mut library_out = ServerLibraryForRead::into_with_role(library, &rights.roles);
                library_out.hidden = hidden;
                if !rights.has_role(&LibraryRole::Admin) {
                    library_out.root = None;
                    library_out.settings = None;
                }
                Some(library_out)
            } else {
                None
            }
        },
        ConnectedUser::Anonymous | ConnectedUser::Guest(_) => None,
        ConnectedUser::ServerAdmin => Some(ServerLibraryForRead::from(library)),
        ConnectedUser::Share(claims) => {
            if claims.kind == ClaimsLocalType::Admin {
                Some(ServerLibraryForRead::from(library))
            } else {
                None
            }
        },
        ConnectedUser::UploadKey(key) => {
            let mut library_out = ServerLibraryForRead::into_with_role(library, &vec![LibraryRole::Write]);
            if library_out.id == key.id {
                library_out.root = None;
                library_out.settings = None;
                Some(library_out)
            } else {
                None
            }
                
        },
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

    pub async fn get_internal_library(&self, library_id: &str, requesting_user: &ConnectedUser) -> Result<Option<super::libraries::ServerLibrary>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Read)?;
		let lib = self.store.get_library(library_id).await?;
		Ok(lib)
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
            self.cache_update_library(library.clone()).await;
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

                ..Default::default()
            };
        self.store.add_library(library).await?;
        let user_id = requesting_user.user_id()?;
        self.store.add_library_rights(library_id.clone(), user_id, vec![LibraryRole::Admin]).await?;
        let library = self.store.get_library(&library_id).await?;
        if let Some(library) = library { 
            self.cache_update_library(library.clone()).await;
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
            self.cache_remove_library(&library.id).await;
            self.store.remove_library(library_id.to_string()).await?;
            self.send_library(LibraryMessage { action: crate::domain::ElementAction::Deleted, library: library.clone() });
            Ok(ServerLibraryForRead::from(library))
        } else {
            Err(Error::NotFound)
        }
	}

    pub async fn add_library_invitation(&self, library_id: &str, roles: Vec<LibraryRole>, requesting_user: &ConnectedUser) -> Result<super::libraries::ServerLibraryInvitation> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let invitation = ServerLibraryInvitation {
            code: nanoid!(),
            expires: None,
            library: library_id.to_string(),
            roles,
        };
        self.store.add_library_invitation(invitation.clone()).await?;
        Ok(invitation)
	}

    

    pub async fn get_watermarks(&self, library_id: &str, requesting_user: &ConnectedUser) -> Result<Vec<String>> {
        requesting_user.check_library_role(&library_id, LibraryRole::Read)?;
        let local = self.library_source_for_library(library_id).await?;
        let path = local.get_gull_path("");
        let mut files = read_dir(&path).await?;
        let mut watermars: Vec<String> = vec![];
        while let Ok(Some(entry)) = files.next_entry().await {
            let metadata = entry.metadata().await?;
            if metadata.is_file() {
                if let Some(filename) = entry.file_name().to_str() {
                    if filename.starts_with(".watermark.") && filename.ends_with(".png") {
                        watermars.push(filename.replace(".watermark.", "").replace(".png", ""));
                    }
                }
            }
        }
     
		Ok(watermars)
	}

    pub async fn get_watermark(&self, library_id: &str, watermark: &str, requesting_user: &ConnectedUser) -> Result<SourceRead> {
        requesting_user.check_library_role(&library_id, LibraryRole::Read)?;

        let watermark = if watermark == "default" {
            "".to_owned()
        } else {
            format!(".{}",watermark)
        };
        let local = self.library_source_for_library(library_id).await?;
        let sourceread = local.get_file(&format!(".watermark{}.png", watermark), None).await?;
            
		Ok(sourceread)
	}


    pub async  fn get_request_share_token(&self, library_id: &str, request: &RsRequest, delay_in_seconds: u64, requesting_user: &ConnectedUser) -> Result<String> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let exp = ClaimsLocal::generate_seconds(delay_in_seconds);
        let claims = ClaimsLocal {
            cr: "service::share_request".to_string(),
            kind: crate::tools::auth::ClaimsLocalType::RequestUrl(request.url.to_string()),
            exp,
        };
        let token = sign_local(claims).await.map_err(|_| Error::UnableToSignShareToken)?;
        Ok(token)
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