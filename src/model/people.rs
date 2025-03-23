


use async_recursion::async_recursion;
use futures::TryStreamExt;
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;
use tokio::{fs::File, io::{AsyncRead, AsyncWriteExt, BufReader}};

use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, url::RsLink, ExternalImage, Gender, ImageType};
use tokio_util::io::StreamReader;
use crate::{domain::{deleted::RsDeleted, library::LibraryRole, people::{PeopleMessage, Person, PersonWithAction}, tag::Tag, ElementAction}, error::{RsError, RsResult}, plugins::sources::{AsyncReadPinBox, FileStreamResult, Source}, tools::{image_tools::{resize_image_reader, ImageSize}, log::log_info}};

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


    
	pub async fn person_image_old(&self, library_id: &str, person_id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        self.library_image(library_id, ".portraits", person_id, kind, size, requesting_user).await
	}


    #[async_recursion]
	pub async fn person_image(&self, library_id: &str, person_id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> crate::Result<FileStreamResult<AsyncReadPinBox>> {
        if RsIds::is_id(person_id) {
            let mut person_ids: RsIds = person_id.to_string().try_into()?;
            let store: std::sync::Arc<super::store::sql::library::SqliteLibraryStore> = self.store.get_library_store(library_id).ok_or(Error::NotFound)?;
            let existing_person = store.get_person_by_external_id(person_ids.clone()).await?;
            if let Some(existing_person) = existing_person {
                let image = self.person_image(library_id, &existing_person.id, kind, size, requesting_user).await?;
                Ok(image)
            } else {

                let local_provider = self.library_source_for_library(library_id).await?;
                if person_ids.tmdb.is_none() {
                    let person = self.trakt.get_person(&person_ids).await?;
                    person_ids = person.into();
                }
                let image_path = format!("cache/person-{}-{}.webp", person_id.replace(':', "-"), kind.as_ref().unwrap_or(&ImageType::Poster));

                if !local_provider.exists(&image_path).await {
                    let images = self.get_person_image_url(&person_ids, kind.as_ref().unwrap_or(&ImageType::Poster), &None).await?.ok_or(crate::Error::NotFound)?;
                    let (_, mut writer) = local_provider.get_file_write_stream(&image_path).await?;
                    let image_reader = reqwest::get(images).await?;
                    let stream = image_reader.bytes_stream();
                    let body_with_io_error = stream.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err));
                    let mut body_reader = StreamReader::new(body_with_io_error);
                    let resized = resize_image_reader(Box::pin(body_reader), ImageSize::Large.to_size(), image::ImageFormat::Avif, Some(70), false).await?;

                    writer.write_all(&resized).await?;
                }

                let source = local_provider.get_file(&image_path, None).await?;
                match source {
                    crate::plugins::sources::SourceRead::Stream(s) => Ok(s),
                    crate::plugins::sources::SourceRead::Request(_) => Err(crate::Error::GenericRedseatError),
                }
            }
        } else {
            if !self.has_library_image(library_id, ".portraits", person_id, kind.clone(), requesting_user).await? {
                log_info(crate::tools::log::LogServiceType::Source, format!("Updating person image: {} {:?}", person_id, kind.clone()));
                self.refresh_person_image(library_id, person_id, &kind, requesting_user).await?;
            }
            
            let image = self.library_image(library_id, ".portraits", person_id, kind, size, requesting_user).await?;
            Ok(image)
        }
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


    /// fetch the plugins to get images for this person
    pub async fn get_person_images(&self, ids: &RsIds) -> RsResult<Vec<ExternalImage>> {
        let mut images = self.tmdb.person_images(ids.clone()).await?;
        Ok(images)
    }
    pub async fn download_person_image(&self, ids: &RsIds, kind: &Option<ImageType>, lang: &Option<String>) -> crate::Result<AsyncReadPinBox> {
        let images = self.get_person_image_url(ids, kind.as_ref().unwrap_or(&ImageType::Poster), lang).await?.ok_or(crate::Error::NotFound)?;
        let image_reader = reqwest::get(images).await?;
        let stream = image_reader.bytes_stream();
        let body_with_io_error = stream.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err));
        let body_reader = StreamReader::new(body_with_io_error);
        Ok(Box::pin(body_reader))
    }    
    pub async fn get_person_image_url(&self, ids: &RsIds, kind: &ImageType, lang: &Option<String>) -> RsResult<Option<String>> {
        let images = if kind == &ImageType::Poster {
            None
        } else { 
            self.tmdb.person_image(ids.clone(), lang).await?.into_kind(kind.clone())
        };
       Ok(images)
    }


    /// download and update image
    pub async fn refresh_person_image(&self, library_id: &str, person_id: &str, kind: &Option<ImageType>, requesting_user: &ConnectedUser) -> RsResult<()> {
        let person = self.get_person(library_id, person_id.to_string(), requesting_user).await?.ok_or(RsError::NotFoundPerson(person_id.to_string()))?;
        let ids: RsIds = person.clone().into();
        let reader = self.download_person_image(&ids, kind, &None).await?;
        self.update_person_image(library_id, person_id, &kind.clone(), reader, &ConnectedUser::ServerAdmin).await?;
        Ok(())
	}

    pub async fn refresh_person(&self, library_id: &str, person_id: &str, requesting_user: &ConnectedUser) -> RsResult<Person> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let person = self.get_person(library_id, person_id.to_string(), requesting_user).await?.ok_or(RsError::NotFoundPerson(person_id.to_string()))?;
        let ids: RsIds = person.clone().into();
        let new_person = self.trakt.get_person(&ids).await?;
        let mut updates = PersonForUpdate {..Default::default()};

        if person.name != new_person.name {
            updates.name = Some(new_person.name);
        }
        if person.bio != new_person.bio {
            updates.bio = new_person.bio;
        }
        if person.imdb != new_person.imdb {
            updates.imdb = new_person.imdb;
        }
        if person.tmdb != new_person.tmdb {
            updates.tmdb = new_person.tmdb;
        }
        if person.slug != new_person.slug {
            updates.slug = new_person.slug;
        }
        if person.birthday != new_person.birthday {
            updates.birthday = new_person.birthday;
        }
        if person.death != new_person.death {
            updates.death = new_person.death;
        }

        let new_person = self.update_person(library_id, person_id.to_string(), updates, requesting_user).await?;
        Ok(new_person)        
	}
    
}
