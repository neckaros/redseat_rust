use std::{collections::HashMap, str::FromStr};


use nanoid::nanoid;
use rs_plugin_common_interfaces::CredentialType;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{domain::credential::Credential, plugins::sources::error::SourcesError};

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
    pub expires: Option<i64>,
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
    pub expires: Option<i64>,
}



impl ModelController {
	pub async fn get_credentials_available(&self, requesting_user: &ConnectedUser) -> Result<HashMap<String, CredentialType>> {
        requesting_user.check_role(&UserRole::Admin)?;
        let mut all_types: HashMap<String, CredentialType> = HashMap::new();
		let all_plugins = &self.plugin_manager.plugins;
        for plugin in all_plugins.read().await.iter() {
            if let Some(cred_type) = plugin.infos.credential_kind.clone() {
                all_types.insert(format!("plugin:{}", plugin.filename.clone()), cred_type);
            }
        }
		Ok(all_types)
	}


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
        let credential = self.store.get_credential(credential_id).await?.ok_or(SourcesError::UnableToFindCredentials("nolib".to_string() ,credential_id.to_string(), "update_credential".to_string()))?;

        Ok(credential)
	}

    pub async fn remove_credential(&self, credential_id: &str, requesting_user: &ConnectedUser) -> Result<Credential> {
        requesting_user.check_role(&UserRole::Admin)?;
        let credential = self.store.get_credential(&credential_id).await?.ok_or(SourcesError::UnableToFindCredentials("nolib".to_string() ,credential_id.to_string(), "update_credential".to_string()))?;

        self.store.remove_credential(credential_id.to_string()).await?;
        Ok(credential)
	}
}


#[cfg(test)]
mod tests {
    use crate::domain::library::LibraryRole;

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