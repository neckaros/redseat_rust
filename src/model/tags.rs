


use std::vec;

use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use x509_parser::nom::branch::alt;


use crate::{domain::{deleted::RsDeleted, library::LibraryRole, tag::{self, Tag, TagMessage, TagWithAction}, ElementAction}, error::RsResult, tools::prediction::PredictionTag};

use super::{error::{Error, Result}, users::ConnectedUser, ModelController};


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TagForAdd {
	pub name: String,
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub alt: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default)]
    pub generated: bool,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TagForInsert {
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


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TagQuery {
    pub name: Option<String>,
    pub parent: Option<String>,
    pub path: Option<String>,
    pub after: Option<u64>
}

impl TagQuery {
    pub fn new_empty() -> TagQuery {
        TagQuery::default()
    }
    pub fn new_with_path(path: String) -> TagQuery {
        TagQuery { path: Some(path), ..Default::default()  }
    }
    pub fn from_after(after: u64) -> TagQuery {
        TagQuery { after: Some(after), ..Default::default()  }
    }

    
    pub fn new_with_name(name: &str) -> TagQuery {
        TagQuery { name: Some(name.to_string()), ..Default::default()  }
    }

    pub fn new_with_name_and_parent(name: &str, parent: Option<String>) -> TagQuery {
        TagQuery { name: Some(name.to_string()), parent, ..Default::default()  }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TagForUpdate {
	pub name: Option<String>,
    pub parent: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    
    pub alt: Option<Vec<String>>,
    pub add_alts: Option<Vec<String>>,
    pub remove_alts: Option<Vec<String>>,

    pub thumb: Option<String>,
    pub params: Option<Value>,
    pub generated: Option<bool>,
    
    pub migrate_to: Option<String>,
}



impl ModelController {

	pub async fn get_tags(&self, library_id: &str, query: TagQuery, requesting_user: &ConnectedUser) -> Result<Vec<Tag>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let tags = store.get_tags(query).await?;
		Ok(tags)
	}

    pub async fn get_ai_tag(&self, library_id: &str, tag: PredictionTag, requesting_user: &ConnectedUser) -> Result<Tag> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let existing_tag = self.get_tag_by_names(&library_id, tag.all_names(), &requesting_user).await?;
        if let Some(existing_tag) = existing_tag {
            return Ok(existing_tag);
        }
        let tag = self.get_or_create_path(&library_id, vec!["ai", &tag.name], TagForUpdate { alt: Some(tag.alts), generated: Some(true), ..Default::default()}, &requesting_user).await?;

		Ok(tag)
	}

    pub async fn get_tag_by_names(&self, library_id: &str, names: Vec<String>, requesting_user: &ConnectedUser) -> Result<Option<Tag>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        for name in names {
            let tag = store.get_tags(TagQuery::new_with_name(&name)).await?.into_iter().nth(0);
            if let Some(tag) = tag {
                return Ok(Some(tag));
            }
        }
		
		Ok(None)
	}

    pub async fn get_tag_by_name(&self, library_id: &str, name: &str, requesting_user: &ConnectedUser) -> Result<Option<Tag>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let tag = store.get_tags(TagQuery::new_with_name(name)).await?.into_iter().nth(0);
		Ok(tag)
	}

    pub async fn get_or_create_path(&self, library_id: &str, mut path: Vec<&str>, template: TagForUpdate, requesting_user: &ConnectedUser) -> Result<Tag> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let path_string = path.join("/");
        let tag_by_path = self.get_tags(&library_id, TagQuery::new_with_path(path_string), &requesting_user).await?.into_iter().nth(0);
        if let Some(tag) = tag_by_path {
            return Ok(tag);
        }
        let mut parent: Option<Tag> = None;

        let last_element = path.pop().ok_or(Error::ServiceError("Empty path".into(), None))?;

        for element in path {
            let previous_parent = parent.as_ref().and_then(|t| Some(t.id.clone()));

            let tag_by_name_and_parent = self.get_tags(&library_id, TagQuery::new_with_name_and_parent(element, previous_parent.clone()), &requesting_user).await?.into_iter().nth(0);
            parent = if let Some(parent) = tag_by_name_and_parent {
                Some(parent.clone())
            } else {
                let new_tag = self.add_tag(&library_id, TagForAdd { name: element.to_string(), parent: previous_parent, generated: template.generated.unwrap_or(false), alt: template.alt.clone(), ..Default::default() } , &requesting_user).await?;
                Some(new_tag)
            }
        }

        let mut all_names = template.alt.clone().unwrap_or(vec![]);
        all_names.insert(0, last_element.to_string());

        for name in all_names {
            let tag_by_name_and_parent = self.get_tags(&library_id, TagQuery::new_with_name_and_parent(&name, parent.as_ref().and_then(|t| Some(t.id.clone()))), &requesting_user).await?.into_iter().nth(0);
            if let Some(tag) = tag_by_name_and_parent {
                return Ok(tag);
            }
        }
        
        let result_tag = self.add_tag(&library_id, TagForAdd { name: last_element.to_string(), parent: parent.and_then(|t| Some(t.id.clone())), generated: template.generated.unwrap_or(false), alt: template.alt.clone(), ..Default::default() } , &requesting_user).await?;

        Ok(result_tag)
	}

    pub fn fill_tags_paths(current_parent: Option<String>, current_path: &str, list: &Vec<Tag>) -> Vec<Tag> {
        let mut output: Vec<Tag> = Vec::new();
        let elements = list.clone().into_iter().filter(|x| x.parent == current_parent).collect::<Vec<Tag>>();
        let remaining_list = list.clone().into_iter().filter(|x| x.parent != current_parent).collect::<Vec<Tag>>();
        for mut element in elements {
            element.path = current_path.to_string();
            output.push(element.clone());
            let mut sub_outout = ModelController::fill_tags_paths(Some(element.id), &format!("{}{}/", &current_path, &element.name), &remaining_list);
            output.append(&mut sub_outout);
        }
        output
    }

    pub async fn get_tag(&self, library_id: &str, tag_id: String, requesting_user: &ConnectedUser) -> Result<Option<Tag>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let tag = store.get_tag(&tag_id).await?;
		Ok(tag)
	}

    pub async fn update_tag(&self, library_id: &str, tag_id: String, update: TagForUpdate, requesting_user: &ConnectedUser) -> Result<Tag> {
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
            self.send_tags(TagMessage { library: library_id.to_string(), tags: all_updated.iter().map(|t| TagWithAction { action: ElementAction::Deleted, tag: t.clone()}).collect()});
            Ok(tag)
        } else {
            Err(Error::NotFound)
        }
	}

    pub async fn merge_tag(&self, library_id: &str, old_id: String, into: String, requesting_user: &ConnectedUser) -> RsResult<Tag> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
		let old_tag = self.get_tag(library_id, old_id.to_owned(), requesting_user).await?.ok_or(Error::TagNotFound(old_id.to_owned()))?;
		let new_tag = self.get_tag(library_id, into.to_owned(), requesting_user).await?.ok_or(Error::TagNotFound(into.to_owned()))?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        if old_tag.name.to_lowercase() != new_tag.name.to_lowercase() {
            store.update_tag(&into, TagForUpdate { add_alts: Some(vec![old_tag.name.to_owned()]), ..Default::default()}).await?
        }
        if let Some(alts) = old_tag.alt {
            store.update_tag(&into, TagForUpdate { add_alts: Some(alts), ..Default::default()}).await?
        }
        store.update_tag(&old_id, TagForUpdate { migrate_to : Some(into.to_owned()), ..Default::default()}).await?;
        self.remove_tag(library_id, &old_id, requesting_user).await?;

        

        let new_tag = self.get_tag(library_id, into, requesting_user).await?.ok_or(Error::TagNotFound(old_id.to_owned()))?;

        self.send_tags(TagMessage { library: library_id.to_string(), tags: vec![TagWithAction { tag: new_tag.clone(), action: ElementAction::Updated}] });

        Ok(new_tag)
	}


	pub fn send_tags(&self, message: TagMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("tags", message);
			}
		});
	}


    pub async fn add_tag(&self, library_id: &str, new_tag: TagForAdd, requesting_user: &ConnectedUser) -> Result<Tag> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let backup = TagForInsert {
            id: nanoid!(),
            name: new_tag.name,
            parent: new_tag.parent,
            kind: new_tag.kind,
            alt: new_tag.alt,
            thumb: new_tag.thumb,
            params: new_tag.params,
            generated: new_tag.generated,
        };
		store.add_tag(backup.clone()).await?;
        let new_tag = self.get_tag(library_id, backup.id, requesting_user).await?.ok_or(Error::NotFound)?;
        self.send_tags(TagMessage { library: library_id.to_string(), tags: vec![TagWithAction { tag: new_tag.clone(), action: ElementAction::Added}] });
		Ok(new_tag)
	}


    pub async fn remove_tag(&self, library_id: &str, tag_id: &str, requesting_user: &ConnectedUser) -> RsResult<Tag> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        let existing = store.get_tag(tag_id).await?;
        if let Some(existing) = existing { 
            let mut children = self.get_tags(library_id, TagQuery::new_with_path(existing.childs_path()), requesting_user).await?;
            children.push(existing.clone());
            store.remove_tag(tag_id.to_string()).await?;
            self.add_deleted(library_id, RsDeleted::serie(tag_id.to_owned()), requesting_user).await?;
            self.send_tags(TagMessage { library: library_id.to_string(), tags: children.iter().map(|t| TagWithAction { action: ElementAction::Deleted, tag: t.clone()}).collect()});
            Ok(existing)
        } else {
            Err(Error::NotFound.into())
        }
	}
    
}
