


use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{fs::File, io::BufReader};

use rs_plugin_url_interfaces::RsLink;
use crate::{domain::{library::LibraryRole, people::{PeopleMessage, Person}, ElementAction}, plugins::sources::{AsyncReadPinBox, FileStreamResult}, tools::image_tools::{ImageSize, ImageType}};

use super::{error::{Error, Result}, users::ConnectedUser, ModelController};


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PersonForAdd {
	pub name: String,
    pub socials: Option<Vec<RsLink>>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub alt: Option<Vec<String>>,
    pub portrait: Option<String>,
    pub params: Option<Value>,
    pub birthday: Option<u64>,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PersonForInsert {
    pub id: String,
	pub name: String,
    pub socials: Option<Vec<RsLink>>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub alt: Option<Vec<String>>,
    pub portrait: Option<String>,
    pub params: Option<Value>,
    pub birthday: Option<u64>,
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PeopleQuery {
    pub after: Option<u64>,
    pub name: Option<String>,
}

impl PeopleQuery {
    pub fn new_empty() -> PeopleQuery {
        PeopleQuery { ..Default::default() }
    }
    pub fn from_after(after: u64) -> PeopleQuery {
        PeopleQuery { after: Some(after), ..Default::default() }
    }
    pub fn from_name(name: &str) -> PeopleQuery {
        PeopleQuery { name: Some(name.to_owned()), ..Default::default() }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PersonForUpdate {
	pub name: Option<String>,
    pub socials: Option<Vec<RsLink>>,
    
    #[serde(rename = "type")]
    pub kind: Option<String>,

    pub alt: Option<Vec<String>>,
    pub add_alts: Option<Vec<String>>,
    pub remove_alts: Option<Vec<String>>,

    
    pub add_socials: Option<Vec<RsLink>>,
    pub remove_socials: Option<Vec<RsLink>>,

    pub portrait: Option<String>,
    pub params: Option<Value>,
    pub birthday: Option<u64>,
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

    pub async fn update_person(&self, library_id: &str, tag_id: String, update: PersonForUpdate, requesting_user: &ConnectedUser) -> Result<Person> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		store.update_person(&tag_id, update).await?;
        let person = store.get_person(&tag_id).await?.ok_or(Error::NotFound)?;
        self.send_people(PeopleMessage { library: library_id.to_string(), action: ElementAction::Updated, people: vec![person.clone()] });
        Ok(person)
	}


	pub fn send_people(&self, message: PeopleMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("tags", message);
			}
		});
	}


    pub async fn add_pesron(&self, library_id: &str, new_person: PersonForAdd, requesting_user: &ConnectedUser) -> Result<Person> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let backup = PersonForInsert {
            id: nanoid!(),
            name: new_person.name,
            socials: new_person.socials,
            kind: new_person.kind,
            alt: new_person.alt,
            portrait: new_person.portrait,
            params: new_person.params,
            birthday: new_person.birthday
        };
		store.add_person(backup.clone()).await?;
        let new_person = self.get_person(library_id, backup.id, requesting_user).await?.ok_or(Error::NotFound)?;
        self.send_people(PeopleMessage { library: library_id.to_string(), action: ElementAction::Added, people: vec![new_person.clone()] });
		Ok(new_person)
	}


    pub async fn remove_person(&self, library_id: &str, tag_id: &str, requesting_user: &ConnectedUser) -> Result<Person> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_person(&tag_id).await?;
        if let Some(existing) = existing { 
            store.remove_person(tag_id.to_string()).await?;
            self.send_people(PeopleMessage { library: library_id.to_string(), action: ElementAction::Removed, people: vec![existing.clone()] });
            Ok(existing)
        } else {
            Err(Error::NotFound)
        }
	}


    
	pub async fn person_image(&self, library_id: &str, person_id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> Result<FileStreamResult<AsyncReadPinBox>> {
        self.library_image(library_id, ".portraits", person_id, kind, size, requesting_user).await
	}

    
}
