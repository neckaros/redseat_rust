use std::str::FromStr;


use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use x509_parser::nom::Err;

use crate::domain::{credential::{self, Credential, CredentialType}, library::LibraryRole};

use super::{error::{Error, Result}, users::{ConnectedUser, UserRole}, ModelController};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CredentialForAdd {
	pub name: String,
	pub source: String,
    #[serde(rename = "type")]
    pub kind: CredentialType,
    pub login: Option<String>,
    pub password: Option<String>,
    pub settings: Value,
    pub user_ref: Option<String>,
    pub refresh_token: Option<String>,
    pub expires: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CredentialForUpdate {
	pub name: Option<String>,
	pub source: Option<String>,
    pub login: Option<String>,
    pub password: Option<String>,
    pub settings: Option<Value>,
    pub user_ref: Option<String>,
    pub refresh_token: Option<String>,
    pub expires: Option<u64>,
}


impl FromStr for CredentialType {
    type Err = Error;
    fn from_str(input: &str) -> Result<CredentialType> {
        match input {
            "oauth"  => Ok(CredentialType::OAuth),
            "token"  => Ok(CredentialType::Token),
            "password"  => Ok(CredentialType::Password),
            _      => Err(Error::UnableToParseEnum),
        }
    }
}

impl core::fmt::Display for CredentialType {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            CredentialType::OAuth  => write!(f, "oauth"),
            CredentialType::Token =>  write!(f, "token"),
            CredentialType::Password =>  write!(f, "password"),
        }
    }
}



impl ModelController {

	pub async fn get_credentials(&self, requesting_user: &ConnectedUser) -> Result<Vec<Credential>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let credentials = self.store.get_credentials().await?;
		Ok(credentials)
	}

    pub async fn get_credential(&self, credential_id: String, requesting_user: &ConnectedUser) -> Result<Option<Credential>> {
        requesting_user.check_role(&UserRole::Admin)?;
		let credential = self.store.get_credential(&credential_id).await?;
		Ok(credential)
	}

    pub async fn add_credential(&self, credential: CredentialForAdd, requesting_user: &ConnectedUser) -> Result<Credential> {
        requesting_user.check_role(&UserRole::Admin)?;
        let credential = Credential {
            id: nanoid!(),
            name: credential.name,
            source: credential.source,
            kind:credential.kind,
            login: credential.login,
            password: credential.password,
            settings: credential.settings,
            user_ref: credential.user_ref,
            refresh_token: credential.refresh_token,
            expires: credential.expires,
        };
		self.store.add_crendential(credential.clone()).await?;
		Ok(credential)
	}

    pub async fn update_credential(&self, credential_id: &str, update: CredentialForUpdate, requesting_user: &ConnectedUser) -> Result<Credential> {
        requesting_user.check_role(&UserRole::Admin)?;
		self.store.update_credentials(credential_id, update).await?;
        let credential = self.store.get_credential(credential_id).await?;
        if let Some(credential) = credential { 
            Ok(credential)
        } else {
            Err(Error::NotFound)
        }
	}

    pub async fn remove_credential(&self, credential_id: &str, requesting_user: &ConnectedUser) -> Result<Credential> {
        requesting_user.check_role(&UserRole::Admin)?;
        let credential = self.store.get_credential(&credential_id).await?;
        if let Some(credential) = credential { 
            self.store.remove_credential(credential_id.to_string()).await?;
            Ok(credential)
        } else {
            Err(Error::NotFound)
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