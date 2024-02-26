


use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;


use crate::domain::{backup::Backup, tag::{Tag, TagMessage}, ElementAction};

use super::{error::{Error, Result}, users::{ConnectedUser, UserRole}, ModelController};


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TagForAdd {
	pub name: String,
    pub parent: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub alt: Vec<String>,
    pub thumb: Option<String>,
    pub params: Option<Value>,
    pub modified: u64,
    pub added: u64,
    pub generated: bool,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TagQuery {
    pub path: Option<String>
}

impl TagQuery {
    pub fn new_empty() -> TagQuery {
        TagQuery { path: None }
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
    pub path: Option<String>,
}



impl ModelController {

	pub async fn get_tags(&self, library_id: &str, query: TagQuery, requesting_user: &ConnectedUser) -> Result<Vec<Tag>> {
        requesting_user.check_role(&UserRole::Read)?;
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
		let tags = store.get_tags(query).await?;
		Ok(tags)
	}

    pub async fn get_tags_with_paths_filling(&self, library_id: &str, requesting_user: &ConnectedUser) -> Result<Vec<Tag>> {
        let tags = self.get_tags(library_id, TagQuery::new_empty(), requesting_user).await?;
        let tags = Self::fill_tags_paths(None, "/", &tags);
		Ok(tags)
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
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        requesting_user.check_role(&UserRole::Admin)?;
		let tag = store.get_tag(&tag_id).await?;
		Ok(tag)
	}

    pub async fn update_tag(&self, library_id: &str, tag_id: String, update: TagForUpdate, requesting_user: &ConnectedUser) -> Result<Tag> {
        let store = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
        requesting_user.check_role(&UserRole::Admin)?;
		store.update_tag(&tag_id, update).await?;
        let tag = store.get_tag(&tag_id).await?;
        if let Some(tag) = tag { 
            self.send_tag(TagMessage { library: library_id.to_string(), action: ElementAction::Updated, tags: vec![tag.clone()] });
            Ok(tag)
        } else {
            Err(Error::NotFound)
        }
	}


	pub fn send_tag(&self, message: TagMessage) {
		self.for_connected_users(&message, |user, socket, message| {
            let r = user.check_library_role(&message.library, &crate::domain::library::LibraryRole::Read);
			if r.is_ok() {
				let _ = socket.emit("library", message);
			}
		});
	}

/*
    pub async fn add_tag(&self, backup: BackupForAdd, requesting_user: &ConnectedUser) -> Result<Backup> {
        requesting_user.check_role(&UserRole::Admin)?;
        let backup = Backup {
            id: nanoid!(),
            source: backup.source,
            credentials: backup.credentials,
            library: backup.library,
            path: backup.path,
            schedule: backup.schedule,
            filter: backup.filter,
            last: backup.last,
            password: backup.password,
            size: backup.size,
        };
		self.store.add_backup(backup.clone()).await?;
		Ok(backup)
	}


    pub async fn remove_tag(&self, backup_id: &str, requesting_user: &ConnectedUser) -> Result<Backup> {
        requesting_user.check_role(&UserRole::Admin)?;
        let credential = self.store.get_backup(&backup_id).await?;
        if let Some(credential) = credential { 
            self.store.remove_backup(backup_id.to_string()).await?;
            Ok(credential)
        } else {
            Err(Error::NotFound)
        }
	}
    */
}
