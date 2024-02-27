


use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;


use crate::domain::{backup::Backup, library::LibraryRole, people::PeopleMessage, tag::{Tag, TagMessage}, ElementAction};

use super::{error::{Error, Result}, users::{ConnectedUser, UserRole}, ModelController};


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PersonForAdd {
	pub name: String,
    pub parent: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub alt: Option<Vec<String>>,
    pub thumb: Option<String>,
    pub params: Option<Value>,
    pub generated: bool,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PersonForInsert {
    pub id: String,
	pub name: String,
    pub parent: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub alt: Option<Vec<String>>,
    pub thumb: Option<String>,
    pub params: Option<Value>,
    pub generated: bool,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PeopleQuery {
    pub after: Option<u64>
}

impl PeopleQuery {
    pub fn new_empty() -> PeopleQuery {
        PeopleQuery { after: None }
    }
    pub fn from_after(after: u64) -> PeopleQuery {
        PeopleQuery { after: Some(after) }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TagForUpdate {
	pub name: Option<String>,
    pub parent: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub alt: Option<Vec<String>>,
    pub thumb: Option<String>,
    pub params: Option<Value>,
    pub generated: Option<bool>,
}



impl ModelController {

	pub async fn get_people(&self, library_id: &str, query: PeopleQuery, requesting_user: &ConnectedUser) -> Result<Vec<Tag>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let tags = store.get_people(query).await?;
		Ok(tags)
	}

    pub async fn get_person(&self, library_id: &str, tag_id: String, requesting_user: &ConnectedUser) -> Result<Option<Tag>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let tag = store.get_tag(&tag_id).await?;
		Ok(tag)
	}

    pub async fn update_person(&self, library_id: &str, tag_id: String, update: TagForUpdate, requesting_user: &ConnectedUser) -> Result<Tag> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		store.update_tag(&tag_id, update.clone()).await?;
        let tag = store.get_tag(&tag_id).await?;
        if let Some(tag) = tag { 
            let mut all_updated = vec![tag.clone()];
            if update.name.is_some() || update.params.is_some() {
                let mut updated = self.get_tags(library_id, TagQuery::new_with_path(format!("{}%",tag.childs_path())), requesting_user).await?;
                all_updated.append(&mut updated);
            }
            self.send_tags(TagMessage { library: library_id.to_string(), action: ElementAction::Updated, tags: all_updated });
            Ok(tag)
        } else {
            Err(Error::NotFound)
        }
	}


	pub fn send_people(&self, message: PeopleMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("tags", message);
			}
		});
	}


    pub async fn add_pesron(&self, library_id: &str, new_tag: PersonForAdd, requesting_user: &ConnectedUser) -> Result<Tag> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let backup = PersonForInsert {
            id: nanoid!(),
            name: new_tag.name,
            parent: new_tag.parent,
            kind: new_tag.kind,
            alt: new_tag.alt,
            thumb: new_tag.thumb,
            params: new_tag.params,
            generated: new_tag.generated,
        };
		store.add_person(backup.clone()).await?;
        let new_tag = self.get_tag(library_id, backup.id, requesting_user).await?.ok_or(Error::NotFound)?;
        self.send_people(PeopleMessage { library: library_id.to_string(), action: ElementAction::Added, people: vec![new_tag.clone()] });
		Ok(new_tag)
	}


    pub async fn remove_person(&self, library_id: &str, tag_id: &str, requesting_user: &ConnectedUser) -> Result<Tag> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_tag(&tag_id).await?;
        if let Some(existing) = existing { 
            let mut children = self.get_tags(library_id, TagQuery::new_with_path(existing.childs_path()), requesting_user).await?;
            children.push(existing.clone());
            store.remove_tag(tag_id.to_string()).await?;
            self.send_tags(TagMessage { library: library_id.to_string(), action: ElementAction::Removed, tags: children });
            Ok(existing)
        } else {
            Err(Error::NotFound)
        }
	}
    
}
