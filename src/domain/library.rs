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
    pub password: Option<String>,

    #[serde(default)]
    pub hidden: bool,
}

impl ServerLibrary {
    pub fn is_virtual(&self) -> bool {
        self.source == "virtual"
    }
}

/// Space of the storage holding a library, in bytes. A provider gives what it knows:
/// one without a quota leaves out `free` and `total`.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStorage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Libraries with the same value share this storage: count its space once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<String>,
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
    Books,
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
    /// Compute media embeddings in the background (semantic search). Off when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeddings: Option<bool>,
    /// Background embedding is suspended until it is resumed. Stored here so a
    /// pause survives a restart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeddings_paused: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_progress: Option<Vec<UserMapping>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_path: Option<String>,

    // IPTV settings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epg_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_streams: Option<u32>,
}

impl ServerLibrarySettings {
    /// Applies a change: the settings the patch does not name are kept.
    pub fn apply(&mut self, patch: ServerLibrarySettingsPatch) {
        patch_setting(&mut self.face_threshold, patch.face_threshold);
        patch_setting(&mut self.ignore_groups, patch.ignore_groups);
        patch_setting(&mut self.preduction_model, patch.preduction_model);
        patch_setting(&mut self.embeddings, patch.embeddings);
        patch_setting(&mut self.embeddings_paused, patch.embeddings_paused);
        patch_setting(&mut self.map_progress, patch.map_progress);
        patch_setting(&mut self.data_path, patch.data_path);
        patch_setting(&mut self.epg_url, patch.epg_url);
        patch_setting(&mut self.max_streams, patch.max_streams);
    }
}

fn patch_setting<T>(setting: &mut Option<T>, change: Option<Option<T>>) {
    if let Some(value) = change {
        *setting = value;
    }
}

/// Change to the settings of a library, read as a JSON Merge Patch (RFC 7396): a
/// setting that is absent is kept, `null` clears it and a value replaces it.
/// An unknown setting is refused instead of being dropped silently.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerLibrarySettingsPatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub face_threshold: Option<Option<f32>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub ignore_groups: Option<Option<bool>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub preduction_model: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub embeddings: Option<Option<bool>>,
    /// Owned by the server: only the pause and resume endpoints change it. What a
    /// client sends is read and dropped, so it can send back the settings it got.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serde_with::rust::double_option::serialize",
        deserialize_with = "ignore_server_owned_setting"
    )]
    pub embeddings_paused: Option<Option<bool>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub map_progress: Option<Option<Vec<UserMapping>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub data_path: Option<Option<String>>,

    // IPTV settings
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub epg_url: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub max_streams: Option<Option<u32>>,
}

impl ServerLibrarySettingsPatch {
    /// Refuses values the server cannot work with. Only what the patch sets is
    /// checked, so a library keeps accepting changes whatever it has stored.
    pub fn validate(&self) -> Result<(), String> {
        // Faces match when their cosine similarity reaches the threshold: at 0 or
        // below every face is the same person, above 1 none ever match.
        if let Some(Some(threshold)) = self.face_threshold {
            if !(threshold > 0.0 && threshold <= 1.0) {
                return Err(format!(
                    "faceThreshold must be above 0 and at most 1, got {threshold}"
                ));
            }
        }
        // No stream could ever start with zero slots.
        if self.max_streams == Some(Some(0)) {
            return Err("maxStreams must be at least 1".to_string());
        }
        Ok(())
    }
}

fn ignore_server_owned_setting<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::de::IgnoredAny::deserialize(deserializer)?;
    Ok(None)
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::{json, Value};

    use super::{LibraryType, ServerLibrarySettings, ServerLibrarySettingsPatch, UserMapping};

    fn patch(value: Value) -> ServerLibrarySettingsPatch {
        serde_json::from_value(value).unwrap()
    }

    /// Every setting filled in. A new setting has to be added here, which makes
    /// `every_setting_can_be_patched` check that the patch knows it too.
    fn full_settings() -> ServerLibrarySettings {
        ServerLibrarySettings {
            face_threshold: Some(0.5),
            ignore_groups: Some(true),
            preduction_model: Some("model".to_string()),
            embeddings: Some(true),
            embeddings_paused: Some(true),
            map_progress: Some(vec![UserMapping {
                from: "user-a".to_string(),
                to: "user-b".to_string(),
            }]),
            data_path: Some("/data".to_string()),
            epg_url: Some("http://epg".to_string()),
            max_streams: Some(2),
        }
    }

    #[test]
    fn settings_patch_keeps_what_it_does_not_name() {
        let mut settings = full_settings();
        settings.apply(patch(json!({ "maxStreams": 4, "epgUrl": null })));

        let mut expected = serde_json::to_value(full_settings()).unwrap();
        expected["maxStreams"] = json!(4);
        expected.as_object_mut().unwrap().remove("epgUrl");
        assert_eq!(serde_json::to_value(&settings).unwrap(), expected);

        settings.apply(patch(json!({})));
        assert_eq!(serde_json::to_value(&settings).unwrap(), expected);
    }

    #[test]
    fn every_setting_can_be_patched() {
        // A client may send back the settings it read: all of them are accepted,
        // and all but the one the server owns are applied.
        let full = serde_json::to_value(full_settings()).unwrap();
        let mut settings = ServerLibrarySettings::default();
        settings.apply(patch(full.clone()));
        let mut expected = full.clone();
        expected.as_object_mut().unwrap().remove("embeddingsPaused");
        assert_eq!(serde_json::to_value(&settings).unwrap(), expected);

        let mut cleared = full.clone();
        for value in cleared.as_object_mut().unwrap().values_mut() {
            *value = Value::Null;
        }
        let mut settings = full_settings();
        settings.apply(patch(cleared));
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            json!({ "embeddingsPaused": true })
        );
    }

    #[test]
    fn settings_patch_refuses_unknown_settings() {
        for value in [
            json!({ "faceTreshold": 0.5 }),
            json!({ "faceTreshold": null }),
        ] {
            let error = serde_json::from_value::<ServerLibrarySettingsPatch>(value).unwrap_err();
            assert!(error.to_string().contains("faceTreshold"), "{error}");
        }
        assert!(serde_json::from_value::<ServerLibrarySettingsPatch>(
            json!({ "maxStreams": "two" })
        )
        .is_err());
    }

    #[test]
    fn settings_patch_from_a_client_cannot_change_the_pause() {
        for sent in [json!(true), json!(false), Value::Null] {
            let from_client = patch(json!({ "embeddingsPaused": sent }));
            assert_eq!(from_client.embeddings_paused, None);
        }

        // The server sets it and clears it through the same patch.
        let mut settings = ServerLibrarySettings::default();
        settings.apply(ServerLibrarySettingsPatch {
            embeddings_paused: Some(Some(true)),
            ..Default::default()
        });
        assert_eq!(settings.embeddings_paused, Some(true));
        settings.apply(ServerLibrarySettingsPatch {
            embeddings_paused: Some(None),
            ..Default::default()
        });
        assert_eq!(settings.embeddings_paused, None);
    }

    #[test]
    fn settings_patch_checks_only_the_values_it_sets() {
        for refused in [
            json!({ "faceThreshold": 0 }),
            json!({ "faceThreshold": -0.2 }),
            json!({ "faceThreshold": 1.5 }),
            json!({ "maxStreams": 0 }),
        ] {
            assert!(patch(refused.clone()).validate().is_err(), "{refused}");
        }
        for accepted in [
            json!({}),
            json!({ "faceThreshold": 0.4, "maxStreams": 1 }),
            json!({ "faceThreshold": 1 }),
            json!({ "faceThreshold": null, "maxStreams": null }),
        ] {
            assert!(patch(accepted.clone()).validate().is_ok(), "{accepted}");
        }
    }

    #[test]
    fn library_type_books_serde_roundtrip() {
        let parsed: LibraryType = serde_json::from_str("\"books\"").unwrap();
        assert_eq!(parsed, LibraryType::Books);

        let serialized = serde_json::to_string(&LibraryType::Books).unwrap();
        assert_eq!(serialized, "\"books\"");
    }

    #[test]
    fn library_type_books_strum_roundtrip() {
        let parsed = LibraryType::from_str("books").unwrap();
        assert_eq!(parsed, LibraryType::Books);
        assert_eq!(LibraryType::Books.to_string(), "books");
    }
}
