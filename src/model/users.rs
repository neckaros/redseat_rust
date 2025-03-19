use std::{cmp::Ordering, str::FromStr};

use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, MediaType, RsRequest};
use rusqlite::{
    types::{FromSql, FromSqlError, FromSqlResult, ValueRef},
    ToSql,
};
use serde::{Deserialize, Serialize};
use strum_macros::EnumString;

use crate::{domain::{library::{LibraryLimits, LibraryRole, LibraryType}, view_progress::{ViewProgress, ViewProgressForAdd}, watched::{Watched, WatchedForAdd}}, error::RsResult, tools::auth::{ClaimsLocal, ClaimsLocalType}};

use super::{error::{Error, Result}, libraries::ServerLibraryForRead, medias::RsSort, store::sql::{users::WatchedQuery, SqlOrder}, ModelController};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub enum ConnectedUser {
    Server(ServerUser),
    Guest(GuestUser),
    Share(ClaimsLocal),
    UploadKey(UploadKey),
    Anonymous,
    ServerAdmin
}

impl ConnectedUser {
    pub fn is_registered(&self) -> bool {
        matches!(&self, ConnectedUser::Server(_)) 
    }
    pub fn check_registered(&self) -> Result<ServerUser> {
        if let ConnectedUser::Server(user) = &self {
            Ok(user.clone())
        } else {
            Err(Error::NotServerConnected)
        }
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
        } else if let ConnectedUser::ServerAdmin = &self {
            Ok("admin".to_string())
        } else if let ConnectedUser::Guest(user) = &self {
            Ok(user.id.clone())
        } else {
            Err(Error::NotServerConnected)
        }
    }
    
    pub fn user_name(&self) -> Result<String> {
        if let ConnectedUser::Server(user) = &self {
            Ok(user.name.clone())
        } else if let ConnectedUser::Guest(user) = &self {
            Ok(user.name.clone())
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
                ClaimsLocalType::RequestUrl(_) => Err(Error::ShareTokenInsufficient),
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
    pub fn check_library_role(&self, library_id: &str, role: LibraryRole) -> Result<LibraryLimits> {
        if self.is_admin() {
            Ok(LibraryLimits::init_with_user(self.user_id().ok()))
        } else if let ConnectedUser::Server(user) = &self {
            if let Some(limits) = user.has_library_role(&library_id, &role) {
                Ok(limits)
            } else {
                Err(Error::InsufficientLibraryRole { user: self.clone(), library_id: library_id.to_string(), role: role.clone() })
            }
        } else if let ConnectedUser::Share(claims) = &self {
            match &claims.kind {
                ClaimsLocalType::File(library, _) => {
                    if library == library_id { 
                        Ok(LibraryLimits::default()) 
                    } else {
                        Err(Error::ShareTokenInsufficient)
                    }
                },
                _ => Err(Error::ShareTokenInsufficient),
            }
        } else if let ConnectedUser::UploadKey(key) = &self {
            if key.library == library_id { 
                Ok(LibraryLimits::default()) 
            } else {
                Err(Error::ShareTokenInsufficient)
            }
        } else {
            Err(Error::NotServerConnected)
        }
    }
    
    pub fn check_file_role(&self, library_id: &str, file_id: &str, role: LibraryRole) -> Result<()> {
        if self.is_admin() {
            Ok(())
        } else if let ConnectedUser::Server(user) = &self {
            if user.has_library_role(&library_id, &role).is_some() {
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
                _ => Err(Error::ShareTokenInsufficient),
            }
        } else {
            Err(Error::NotServerConnected)
        }
    }
    pub fn check_request_role(&self, library_id: &str, request: &RsRequest) -> Result<()> {
        if self.is_admin() {
            Ok(())
        } else if let ConnectedUser::Server(user) = &self {
            if user.has_library_role(library_id, &LibraryRole::Read).is_some() {
                Ok(())
            } else {
                Err(Error::InsufficientLibraryRole { user: self.clone(), library_id: library_id.to_string(), role: LibraryRole::Read })
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
                ClaimsLocalType::RequestUrl(url) => {
                    if *url == request.url { 
                        Ok(()) 
                    } else {
                        Err(Error::ShareTokenInsufficient)
                    }
                },
                _ => Err(Error::ShareTokenInsufficient),
            }
        } else {
            Err(Error::NotServerConnected)
        }
    }

    pub fn has_hidden_library(&self, library_id: String) -> bool {
        match self {
            ConnectedUser::Server(user) => user.preferences.hidden_libraries.contains(&library_id),
            _ => false,
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
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
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
    pub limits: LibraryLimits,
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
    pub limits: LibraryLimits,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerLibrariesRightsForAdd {
    pub library_id: String,
    pub user_id: String,
    pub roles: Vec<LibraryRole>,
    pub limits: LibraryLimits,
}



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct ServerUser {
    pub id: String,
    pub name: String,
    pub role: UserRole,
    pub preferences: ServerUserPreferences,
    pub libraries: Vec<ServerUserLibrariesRights>
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct GuestUser {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct UploadKey {
    pub id: String,
    pub library: String,
    pub expiry: Option<i64>,
    #[serde(default)]
    pub tags: bool,
}

impl ServerUser {
    pub fn is_admin(&self) -> bool {
        matches!(&self.role, UserRole::Admin)
    }
    pub fn has_role(&self, role: &UserRole) -> bool {
        &self.role >= role
    }
    pub fn has_library_role(&self, library_id: &str, role: &LibraryRole) -> Option<LibraryLimits> {
        let libraries = &self.libraries.clone();
        let found = libraries.into_iter().find(|l| l.id == library_id);
        if let Some(found) = found {
            if found.has_role(role) {
                Some(found.limits.clone())
            } else {
                None
            }
        } else {
            None
        }
    }
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct InvitationRedeemer {
	pub code: String,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerUserForUpdate {
    pub id: String,
    pub name: Option<String>,
    pub role: Option<UserRole>,
    pub preferences: Option<ServerUserPreferences>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct HistoryQuery {
    
    #[serde(default)]
    pub sort: RsSort,
    #[serde(default)]
    pub order: SqlOrder,

    pub before: Option<i64>,
    pub after: Option<i64>,
    #[serde(default)]
    pub types: Vec<MediaType>,

    pub id: Option<RsIds>,
    
    pub page_key: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")] 
pub struct ViewProgressQuery {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
}




impl ModelController {
    pub async fn get_watched(&self, query: HistoryQuery, user: &ConnectedUser, library_id: Option<String>) -> RsResult<Vec<Watched>> {
        user.check_role(&UserRole::Read)?;
        if matches!(user, ConnectedUser::ServerAdmin) {
            return Ok(vec![])
        }
        let user_id = user.user_id()?;
        let watcheds = self.store.get_watched( query, user_id, vec![]).await?;
        Ok(watcheds)       
    }

    pub async fn add_watched(&self, watched: WatchedForAdd, user: &ConnectedUser, library_id: Option<String>) -> RsResult<()> {
        user.check_role(&UserRole::Read)?;

        let user_id = user.user_id()?;
        if let Some(library_id) = library_id {
            let all_ids = self.get_library_progress_merged_users(&library_id, user_id).await?;
            for id in all_ids {
                self.store.add_watched(watched.clone(), id).await?;
            }
        } else {
            self.store.add_watched(watched.clone(), user_id).await?;
        }
        Ok(())
    }

    pub async fn add_view_progress(&self, progress: ViewProgressForAdd, user: &ConnectedUser, library_id: Option<String>) -> RsResult<()> {
        user.check_role(&UserRole::Read)?;

        let user_id = user.user_id()?;
        if let Some(library_id) = library_id {
            let all_ids = self.get_library_progress_merged_users(&library_id, user_id).await?;
            for id in all_ids {
                self.store.add_view_progress(progress.clone(), id).await?;
            }
        } else {
            self.store.add_view_progress(progress.clone(), user_id).await?;
        }
        Ok(())
    }

    pub async fn get_view_progress(&self, ids: RsIds, user: &ConnectedUser, library_id: Option<String>) -> RsResult<Option<ViewProgress>> {
        if matches!(user, ConnectedUser::ServerAdmin) {
            return Ok(None)
        }


        user.check_role(&UserRole::Read)?;
        let user_id = user.user_id()?;
        let progress = self.store.get_view_progess( ids, user_id.clone()).await?;
        Ok(progress)
    }

    pub async fn get_all_view_progress(&self, query: HistoryQuery, user: &ConnectedUser, library_id: Option<String>) -> RsResult<Vec<ViewProgress>> {
        if matches!(user, ConnectedUser::ServerAdmin) {
            return Ok(vec![])
        }


        user.check_role(&UserRole::Read)?;
        let user_id = user.user_id()?;
        let progresses = self.store.get_all_view_progress(query, user_id).await?;
        Ok(progresses)
    }

    pub async fn get_view_progress_by_id(&self, id: String, user: &ConnectedUser) -> RsResult<Option<ViewProgress>> {
        if matches!(user, ConnectedUser::ServerAdmin) {
            return Ok(None)
        }

        user.check_role(&UserRole::Read)?;
        let media_id = RsIds::try_from(id)?;
        let progress = match user {
            ConnectedUser::Server(user) => self.store.get_view_progess( media_id, user.id.clone()).await?,
            _ => None
        };
        Ok(progress)
    }

    pub async fn get_upload_key(&self, key: String) -> RsResult<UploadKey> {
        Ok(self.store.get_upload_key(key).await?)
    }


    pub async fn redeem_invitation(&self, code: String, user: ConnectedUser) -> RsResult<String> {
        let connected_user = match user {
            ConnectedUser::Server(u) => Ok(u),
            ConnectedUser::Guest(u) => {
                let creation_user = ServerUser {
                    id: u.id,
                    name: u.name,
                    role: UserRole::Read,
			        ..Default::default()
                };
                self.add_user(creation_user, &ConnectedUser::ServerAdmin).await
            },
            _ => Err(Error::UserGetNotAuth { user: user.clone(), requested_user: "Connected".to_string() }),
        }?;
        let invitation = self.store.get_library_invitation(code.clone()).await?.ok_or(Error::NotFound)?;
        let library_id = invitation.library.clone();
        self.store.add_library_rights(invitation.library, connected_user.id, invitation.roles, invitation.limits).await?;

        self.store.remove_library_invitation(code).await?;

        Ok(library_id)
    }


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