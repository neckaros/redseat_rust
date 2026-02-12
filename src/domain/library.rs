use rusqlite::{
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    ToSql,
};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

use super::ElementAction;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ServerLibrary {
    pub id: String,
    pub name: String,
    pub source: String,
    pub root: Option<String>,
    #[serde(rename = "type")]
    pub kind: LibraryType,
    pub crypt: Option<bool>,
    pub settings: ServerLibrarySettings,
    pub credentials: Option<String>,
    pub plugin: Option<String>,

    #[serde(default)]
    pub hidden: bool,
}

impl ServerLibrary {
    pub fn is_virtual(&self) -> bool {
        self.source == "virtual"
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, EnumString, Default)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum LibraryRole {
    Admin,
    Read,
    Write,
    #[default]
    None,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct LibraryLimits {
    #[serde(default)]
    pub tags: bool,
    #[serde(default)]
    pub people: bool,
    #[serde(default)]
    pub albums: bool,
    #[serde(default)]
    pub delay: Option<i64>,
    #[serde(default)]
    pub user_id: Option<String>,
}

impl LibraryLimits {
    pub fn init_with_user(user_id: Option<String>) -> Self {
        let mut limits = LibraryLimits::default();
        limits.user_id = user_id;
        limits
    }
}

impl FromSql for LibraryLimits {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            let r = serde_json::from_str(&as_string).map_err(|_| FromSqlError::InvalidType);
            r
        })
    }
}

impl ToSql for LibraryLimits {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let r = serde_json::to_string(self).map_err(|_| FromSqlError::InvalidType)?;
        Ok(ToSqlOutput::from(r))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, EnumString, Display)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum LibraryType {
    Photos,
    Shows,
    Movies,
    Iptv,
    #[default]
    Other,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UserMapping {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerLibrarySettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face_threshold: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_groups: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preduction_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_progress: Option<Vec<UserMapping>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMessage {
    pub action: ElementAction,
    pub library: ServerLibrary,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatusMessage {
    pub message: String,
    pub library: String,
}
