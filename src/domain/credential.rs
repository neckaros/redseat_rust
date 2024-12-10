use rs_plugin_common_interfaces::{CredentialType, PluginCredential};
use serde::{Deserialize, Serialize};
use serde_json::Value;


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Credential {
    pub id: String,
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

impl From<Credential> for PluginCredential {
    fn from(credential: Credential) -> Self {
        PluginCredential {
            kind: credential.kind,
            login: credential.login,
            password: credential.password,
            settings: credential.settings,
            user_ref: credential.user_ref,
            refresh_token: credential.refresh_token,
            expires: credential.expires,
        } 
    }
}