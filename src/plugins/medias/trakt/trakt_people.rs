use std::{collections::HashMap, iter::Map};

use chrono::{DateTime, Utc};
use rs_plugin_common_interfaces::{url::ToRsLinks, Gender, RsLink};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};
use crate::{domain::{episode::Episode, people::Person}, tools::clock::deserialize_optional_date_as_ms_timestamp};

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
    pub person: TraktPerson
}

