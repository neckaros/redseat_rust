


use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;
use tokio::{fs::File, io::{AsyncRead, BufReader}};

use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, url::RsLink, Gender, ImageType};
use crate::{domain::{deleted::RsDeleted, library::LibraryRole, people::{PeopleMessage, Person, PersonWithAction}, tag::Tag, ElementAction}, error::RsResult, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::image_tools::ImageSize};

use super::{error::{Error, Result}, users::ConnectedUser, ModelController};


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PersonForAdd {
	pub name: String,
    pub socials: Option<Vec<RsLink>>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub alt: Option<Vec<String>>,
    pub portrait: Option<String>,
    pub params: Option<Value>,
    pub birthday: Option<i64>,
    #[serde(default)]
    pub generated: bool,

    pub imdb: Option<String>,
    pub slug: Option<String>,
    pub tmdb: Option<u64>,
    pub trakt: Option<u64>,

        
    pub death: Option<i64>,
    pub gender: Option<Gender>,
    pub country: Option<String>,
    pub bio: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PersonForInsert {
    pub id: String,
	pub person: PersonForAdd
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PeopleQuery {
    pub after: Option<i64>,
    pub name: Option<String>,
}

impl PeopleQuery {
    pub fn new_empty() -> PeopleQuery {
        PeopleQuery { ..Default::default() }
    }
    pub fn from_after(after: i64) -> PeopleQuery {
        PeopleQuery { after: Some(after), ..Default::default() }
    }
    pub fn from_name(name: &str) -> PeopleQuery {
        PeopleQuery { name: Some(name.to_owned()), ..Default::default() }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonForUpdate {
	pub name: Option<String>,
    pub socials: Option<Vec<RsLink>>,
    
    #[serde(rename = "type")]
    pub kind: Option<String>,

    pub alt: Option<Vec<String>>,
    pub add_alts: Option<Vec<String>>,
    pub remove_alts: Option<Vec<String>>,

    pub add_social_url: Option<String>,
    pub add_socials: Option<Vec<RsLink>>,
    pub remove_socials: Option<Vec<RsLink>>,

    pub portrait: Option<String>,
    pub params: Option<Value>,
    pub birthday: Option<i64>,
    pub generated: Option<bool>,

    pub imdb: Option<String>,
    pub slug: Option<String>,
    pub tmdb: Option<u64>,
    pub trakt: Option<u64>,
    
    pub death: Option<i64>,
    pub gender: Option<Gender>,
    pub country: Option<String>,
    pub bio: Option<String>,
}



impl ModelController {

	pub async fn get_people(&self, library_id: &str, query: PeopleQuery, requesting_user: &ConnectedUser) -> Result<Vec<Person>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let people = store.get_people(query).await?;
		Ok(people)
	}

    pub async fn get_person(&self, library_id: &str, tag_id: String, requesting_user: &ConnectedUser) -> Result<Option<Person>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let tag = store.get_person(&tag_id).await?;
		Ok(tag)
	}

    pub async fn update_person(&self, library_id: &str, tag_id: String, mut update: PersonForUpdate, requesting_user: &ConnectedUser) -> RsResult<Person> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        if let Some(origin) = &update.add_social_url {
            let mut new_socials = update.add_socials.unwrap_or_default();
            new_socials.push(self.exec_parse(Some(library_id.to_owned()), origin.to_owned(), requesting_user).await?);
            update.add_socials = Some(new_socials);
        }
        println!("socialts {:?}", update);
		store.update_person(&tag_id, update).await?;
        let person = store.get_person(&tag_id).await?.ok_or(Error::NotFound)?;
        self.send_people(PeopleMessage { library: library_id.to_string(), people: vec![PersonWithAction { person: person.clone(), action: ElementAction::Updated}] });
        Ok(person)
	}


	pub fn send_people(&self, message: PeopleMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("people", message);
			}
		});
	}


    pub async fn add_pesron(&self, library_id: &str, new_person: PersonForAdd, requesting_user: &ConnectedUser) -> Result<Person> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let backup = PersonForInsert {
            id: nanoid!(),
            person: new_person
        };
		store.add_person(backup.clone()).await?;
        let new_person = self.get_person(library_id, backup.id, requesting_user).await?.ok_or(Error::NotFound)?;
        self.send_people(PeopleMessage { library: library_id.to_string(), people: vec![PersonWithAction { person: new_person.clone(), action: ElementAction::Added}] });
		Ok(new_person)
	}


    pub async fn remove_person(&self, library_id: &str, tag_id: &str, requesting_user: &ConnectedUser) -> RsResult<Person> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_person(tag_id).await?;
        if let Some(existing) = existing { 
            store.remove_person(tag_id.to_string()).await?;
            self.add_deleted(library_id, RsDeleted::person(tag_id.to_owned()), requesting_user).await?;
            self.send_people(PeopleMessage { library: library_id.to_string(), people: vec![PersonWithAction { person: existing.clone(), action: ElementAction::Deleted}] });
            Ok(existing)
        } else {
            Err(Error::NotFound.into())
        }
	}


    
	pub async fn person_image(&self, library_id: &str, person_id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        self.library_image(library_id, ".portraits", person_id, kind, size, requesting_user).await
	}

    pub async fn update_person_image<T: AsyncRead>(&self, library_id: &str, person_id: &str, kind: &Option<ImageType>, reader: T, requesting_user: &ConnectedUser) -> Result<Person> {
        if RsIds::is_id(&person_id) {
            return Err(Error::InvalidIdForAction("udpate person image".to_string(), person_id.to_string()))
        }
        self.update_library_image(library_id, ".portraits", person_id, kind, reader, requesting_user).await?;
        
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        store.update_person_portrait(person_id.to_string()).await?;
        let person = self.get_person(library_id, person_id.to_owned(), requesting_user).await?.ok_or(Error::PersonNotFound(person_id.to_owned()))?;
        self.send_people(PeopleMessage { library: library_id.to_string(), people: vec![PersonWithAction { person: person.clone(), action: ElementAction::Updated}] });
        Ok(person)
	}

    
}
