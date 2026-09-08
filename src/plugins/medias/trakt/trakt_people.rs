use std::{collections::HashMap, iter::Map};

use crate::{
    domain::{episode::Episode, people::Person},
    tools::clock::deserialize_optional_date_as_ms_timestamp,
};
use chrono::{DateTime, Utc};
use rs_plugin_common_interfaces::{url::ToRsLinks, Gender, RsLink};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

use super::trakt_show::TraktIds;

#[derive(Debug, Serialize, Deserialize, EnumString, Display, Default)]
#[serde(rename_all = "lowercase")]
pub enum TraktGender {
    #[serde(rename = "male")]
    Male,
    #[serde(rename = "female")]
    Female,
    #[serde(rename = "non_binary")]
    NonBinary,
    #[strum(default)]
    Other(String),
    #[default]
    Unknown,
}

impl From<TraktGender> for Gender {
    fn from(value: TraktGender) -> Self {
        match value {
            TraktGender::Male => Gender::Male,
            TraktGender::Female => Gender::Female,
            TraktGender::NonBinary => Gender::Other,
            TraktGender::Other(s) => Gender::Unknown,
            TraktGender::Unknown => Gender::Unknown,
        }
    }
}

/// An [episode] with full [extended info]
///
/// [episode]: https://trakt.docs.apiary.io/#reference/episodes
/// [extended info]: https://trakt.docs.apiary.io/#introduction/extended-info
#[derive(Debug, Serialize, Deserialize)]
pub struct TraktPerson {
    pub name: String,
    pub ids: TraktIds,
    pub social_ids: Option<HashMap<String, Option<String>>>,

    pub biography: Option<String>,

    #[serde(deserialize_with = "deserialize_optional_date_as_ms_timestamp")]
    pub birthday: Option<i64>,
    #[serde(deserialize_with = "deserialize_optional_date_as_ms_timestamp")]
    pub death: Option<i64>,

    pub birthplace: Option<String>,
    pub homepage: Option<String>,
    pub gender: Option<TraktGender>,
    pub known_for_department: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<TraktPerson> for Person {
    fn from(value: TraktPerson) -> Self {
        let socials = if let Some(homepage) = value.homepage {
            let mut socials = value.social_ids.to_rs_links();
            socials.push(RsLink {
                platform: "link".to_string(),
                id: homepage,
                ..Default::default()
            });
            Some(socials)
        } else {
            value.social_ids.map(|s| s.to_rs_links())
        };
        Person {
            id: format!("trakt:{}", value.ids.trakt.unwrap()),
            name: value.name,
            bio: value.biography,
            birthday: value.birthday,
            death: value.death,
            country: value.birthplace,
            gender: value.gender.map(Gender::from),
            imdb: value.ids.imdb,
            slug: value.ids.slug,
            tmdb: value.ids.tmdb,
            trakt: value.ids.trakt,
            socials,
            kind: value.known_for_department.map(|v| v.to_string()),

            ..Default::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TraktActorsResult {
    pub cast: Vec<TraktCast>,
    pub crew: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TraktCast {
    pub character: String,
    pub characters: Vec<String>,
    pub person: TraktPerson,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TraktCrewList {
    pub production: String,
    pub directing: Vec<String>,
    pub writing: TraktPerson,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct TraktCrew {
    pub job: String,
    pub jobs: Vec<String>,
    pub person: TraktPerson,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TraktPeopleSearchElement {
    pub score: f64,
    pub person: TraktPerson,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::people::PersonForUpdate;
    #[test]
    fn metadata_refresh_person_converts_and_updates_provider_fields() {
        let raw: TraktPerson = serde_json::from_value(serde_json::json!({
            "name":"Person", "ids":{"trakt":42}, "birthday":"1980-01-01", "death":"2020-01-01",
            "birthplace":"France", "gender":"female", "known_for_department":"Acting", "homepage":"https://example.com"
        })).unwrap();
        let person: Person = raw.into();
        assert!(person.birthday.is_some());
        assert!(person.death.is_some());
        let update = PersonForUpdate::from_refresh(&Person::default(), person);
        assert_eq!(update.country.as_deref(), Some("France"));
        assert_eq!(update.gender, Some(Gender::Female));
        assert_eq!(update.trakt, Some(42));
        assert!(update.birthday.is_some());
        assert!(update.death.is_some());
        assert_eq!(update.kind.as_deref(), Some("Acting"));
        assert!(!update.add_socials.unwrap().is_empty());
    }
    #[tokio::test]
    async fn metadata_refresh_person_preserves_stored_socials() {
        use crate::model::store::sql::library::SqliteLibraryStore;
        let connection = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        let store = SqliteLibraryStore::new(connection).await.unwrap();
        let manual = RsLink {
            platform: "link".into(),
            id: "https://manual.example.com".into(),
            ..Default::default()
        };
        let provider = RsLink {
            platform: "link".into(),
            id: "https://provider.example.com".into(),
            ..Default::default()
        };
        let stored = Person {
            id: "refresh-person".into(),
            name: "Person".into(),
            socials: Some(vec![manual.clone()]),
            ..Default::default()
        };
        store
            .add_person(crate::model::people::PersonForInsert {
                id: stored.id.clone(),
                person: crate::model::people::PersonForAdd {
                    name: stored.name.clone(),
                    socials: stored.socials.clone(),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        let incoming = Person {
            socials: Some(vec![provider.clone(), provider.clone()]),
            ..stored.clone()
        };
        let update = PersonForUpdate::from_refresh(&stored, incoming);
        assert!(update.socials.is_none());
        assert_eq!(
            update.add_socials.as_ref().unwrap(),
            &vec![provider.clone()]
        );
        store.update_person(&stored.id, update).await.unwrap();
        let loaded = store.get_person(&stored.id).await.unwrap().unwrap();
        assert_eq!(
            loaded.socials.as_ref().unwrap(),
            &vec![manual, provider.clone()]
        );
        for socials in [None, Some(vec![]), Some(vec![provider])] {
            let incoming = Person {
                socials,
                ..loaded.clone()
            };
            let update = PersonForUpdate::from_refresh(&loaded, incoming);
            assert!(update.socials.is_none());
            assert!(update.add_socials.is_none());
        }
    }
}
