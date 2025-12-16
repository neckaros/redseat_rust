
use std::{collections::HashMap, io::Cursor, sync::{Arc, Mutex}};

use async_recursion::async_recursion;
use futures::TryStreamExt;
use lazy_static::lazy_static;
use nanoid::nanoid;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::EnumString;
use tokio::{fs::File, io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader}};

use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, url::RsLink, ExternalImage, Gender, ImageType};
use tokio_util::io::StreamReader;
use crate::{domain::{deleted::RsDeleted, library::LibraryRole, media::{FileType, Media, MediaWithAction, MediasMessage}, people::{FaceBBox, FaceEmbedding, PeopleMessage, Person, PersonWithAction, UnassignedFace}, tag::Tag, ElementAction}, error::{RsError, RsResult}, model::medias::MediaFileQuery, plugins::sources::{error::SourcesError, AsyncReadPinBox, FileStreamResult, Source}, tools::{image_tools::{convert_image_reader, resize_image_reader, ImageSize}, log::log_info, recognition::{BBox, DetectedFace, FaceRecognitionService}, video_tools::VideoTime}};

use super::{error::{Error, Result}, users::ConnectedUser, ModelController};

// Default face recognition threshold when library doesn't specify one
pub const DEFAULT_FACE_THRESHOLD: f32 = 0.4;

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

lazy_static! {
    static ref FACE_RECOGNITION_SERVICE: Mutex<Option<Arc<FaceRecognitionService>>> = Mutex::new(None);
}

#[derive(Serialize)]
pub struct DetectedFaceResult {
    pub face_id: Option<String>,
    pub confidence: f32,
    pub bbox: FaceBBox,
}

#[derive(Serialize)]
pub struct ClusteringResult {
    pub clusters_created: usize,
}

impl ModelController {

    async fn get_face_recognition_service(&self) -> RsResult<Arc<FaceRecognitionService>> {
        {
            let guard = FACE_RECOGNITION_SERVICE.lock().unwrap();
            if let Some(service) = &*guard {
                return Ok(service.clone());
            }
        } 
    
        let service = FaceRecognitionService::new_async("models").await?;
        
        let mut guard = FACE_RECOGNITION_SERVICE.lock().unwrap();
        if let Some(existing) = &*guard {
            return Ok(existing.clone());
        }
        
        let service = Arc::new(service);
        *guard = Some(service.clone());
        Ok(service)
    }

    /// Get the face recognition threshold for a library.
    /// Returns library.settings.face_threshold if set, otherwise DEFAULT_FACE_THRESHOLD.
    async fn get_face_threshold(&self, library_id: &str) -> RsResult<f32> {
        let library = self.get_internal_library(library_id).await?
            .ok_or_else(|| RsError::Error(format!("Library not found: {}", library_id)))?;
        
        Ok(library.settings.face_threshold.unwrap_or(DEFAULT_FACE_THRESHOLD))
    }

	pub async fn get_people(&self, library_id: &str, query: PeopleQuery, requesting_user: &ConnectedUser) -> Result<Vec<Person>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(Error::LibraryStoreNotFoundFor(library_id.to_string(), "get_people".to_string()))?;
		let people = store.get_people(query).await?;
		Ok(people)
	}

    pub async fn get_person(&self, library_id: &str, tag_id: String, requesting_user: &ConnectedUser) -> Result<Option<Person>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(Error::LibraryStoreNotFoundFor(library_id.to_string(), "get_person".to_string()))?;
		let tag = store.get_person(&tag_id).await?;
		Ok(tag)
	}

    pub async fn update_person(&self, library_id: &str, tag_id: String, mut update: PersonForUpdate, requesting_user: &ConnectedUser) -> RsResult<Person> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(Error::LibraryStoreNotFoundFor(library_id.to_string(), "update_person".to_string()))?;
        if let Some(origin) = &update.add_social_url {
            let mut new_socials = update.add_socials.unwrap_or_default();
            new_socials.push(self.exec_parse(Some(library_id.to_owned()), origin.to_owned(), requesting_user).await?);
            update.add_socials = Some(new_socials);
        }
		store.update_person(&tag_id, update).await?;
        let person = store.get_person(&tag_id).await?.ok_or(SourcesError::UnableToFindPerson(library_id.to_string(), tag_id.to_string(), "update_person".to_string()))?;
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
        let store = self.store.get_library_store_optional(library_id).ok_or(Error::LibraryStoreNotFoundFor(library_id.to_string(), "add_pesron".to_string()))?;
        let backup = PersonForInsert {
            id: nanoid!(),
            person: new_person
        };
		store.add_person(backup.clone()).await?;
        let new_person = self.get_person(library_id, backup.id.clone(), requesting_user).await?.ok_or(SourcesError::UnableToFindPerson(library_id.to_string(), backup.id, "add_pesron".to_string()))?;
        self.send_people(PeopleMessage { library: library_id.to_string(), people: vec![PersonWithAction { person: new_person.clone(), action: ElementAction::Added}] });
		Ok(new_person)
	}


    pub async fn remove_person(&self, library_id: &str, tag_id: &str, requesting_user: &ConnectedUser) -> RsResult<Person> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        let store = self.store.get_library_store_optional(library_id).ok_or(Error::LibraryStoreNotFoundFor(library_id.to_string(), "remove_person".to_string()))?;
        let existing = store.get_person(tag_id).await?.ok_or(SourcesError::UnableToFindPerson(library_id.to_string(), tag_id.to_string(), "remove_person".to_string()))?;
        
        store.remove_person(tag_id.to_string()).await?;
        self.add_deleted(library_id, RsDeleted::person(tag_id.to_owned()), requesting_user).await?;
        self.send_people(PeopleMessage { library: library_id.to_string(), people: vec![PersonWithAction { person: existing.clone(), action: ElementAction::Deleted}] });
        Ok(existing)
	}


    
	pub async fn person_image_old(&self, library_id: &str, person_id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> RsResult<FileStreamResult<AsyncReadPinBox>> {
        self.library_image(library_id, ".portraits", person_id, kind, size, requesting_user).await
	}


    #[async_recursion]
	pub async fn person_image(&self, library_id: &str, person_id: &str, kind: Option<ImageType>, size: Option<ImageSize>, requesting_user: &ConnectedUser) -> crate::Result<FileStreamResult<AsyncReadPinBox>> {
        if RsIds::is_id(person_id) {
            let mut person_ids: RsIds = person_id.to_string().try_into()?;
            let store = self.store.get_library_store_optional(library_id).ok_or(SourcesError::UnableToFindPerson(library_id.to_string(), person_id.to_string(), "person_image".to_string()))?;
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
                let image_path = format!("cache/person-{}-{}.avif", person_id.replace(':', "-"), kind.as_ref().unwrap_or(&ImageType::Poster));

                if !local_provider.exists(&image_path).await {
                    let images = self.get_person_image_url(&person_ids, kind.as_ref().unwrap_or(&ImageType::Poster), &None).await?.ok_or(crate::Error::NotFound(format!("Unable to get person image url: {:?} kind {:?}",person_ids, kind)))?;
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
                // Try to refresh from external source first
                let refresh_result = self.refresh_person_image(library_id, person_id, &kind, requesting_user).await;
                match refresh_result {
                    Ok(_) => {
                        // External image refresh succeeded, image should now exist
                        log_info(crate::tools::log::LogServiceType::Source, format!("Successfully refreshed person image from external source: {} {:?}", person_id, kind.clone()));
                    }
                    Err(_) => {
                        // External image refresh failed, try face fallback
                        let store = self.store.get_library_store_optional(library_id).ok_or(SourcesError::UnableToFindPerson(library_id.to_string(), person_id.to_string(), "person_image".to_string()))?;
                        if let Ok(Some(face)) = store.get_highest_confidence_face(person_id).await {
                            // Check that face has required fields
                            if let (Some(media_ref), Some(_bbox)) = (face.media_ref.as_ref(), face.bbox.as_ref()) {
                                if !media_ref.is_empty() {
                                    // Extract face image and save as person image
                                    log_info(crate::tools::log::LogServiceType::Source, format!("Updating person ({}) image from face: {}", person_id, face.id));
                                    match self.get_face_image(library_id, &face.id, &ConnectedUser::ServerAdmin).await {
                                        Ok(face_bytes) => {
                                            let reader = Cursor::new(face_bytes);
                                            match self.update_person_image(library_id, person_id, &kind, reader, &ConnectedUser::ServerAdmin).await {
                                                Ok(_) => {
                                                    log_info(crate::tools::log::LogServiceType::Source, format!("Successfully saved face image as person image for: {}", person_id));
                                                }
                                                Err(e) => {
                                                    log_info(crate::tools::log::LogServiceType::Source, format!("Failed to save face image as person image: {}", e));
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            log_info(crate::tools::log::LogServiceType::Source, format!("Failed to get face image: {}", e));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            let image = self.library_image(library_id, ".portraits", person_id, kind, size, requesting_user).await?;
            Ok(image)
        }
	}

    pub async fn update_person_image<T: AsyncRead>(&self, library_id: &str, person_id: &str, kind: &Option<ImageType>, reader: T, requesting_user: &ConnectedUser) -> RsResult<Person> {
        if RsIds::is_id(&person_id) {
            return Err(Error::InvalidIdForAction("udpate person image".to_string(), person_id.to_string()).into())
        }
        self.update_library_image(library_id, ".portraits", person_id, kind, &None, reader, requesting_user).await?;
        
        let store = self.store.get_library_store(library_id)?;
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
        let images = self.get_person_image_url(ids, kind.as_ref().unwrap_or(&ImageType::Poster), lang).await?.ok_or(crate::Error::NotFound(format!("Unable to get person image url: {:?} kind {:?}",ids, kind)))?;
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

    // FACE RECOGNITION

    pub async fn process_media_faces(
        &self,
        library_id: &str,
        media_id: &str,
        requesting_user: &ConnectedUser
    ) -> RsResult<Vec<DetectedFaceResult>> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id)?;
        
        // Get media to check type
        let media = self.get_media(library_id, media_id.to_string(), requesting_user).await?
            .ok_or(SourcesError::UnableToFindMedia(library_id.to_string(), media_id.to_string(), "process_media_faces".to_string()))?;
        
        // Load images based on media type
        let images_data = match media.kind {
            FileType::Photo => {
                // Load full image for photos
                let reader = self.library_file(library_id, media_id, None, MediaFileQuery::default(), requesting_user).await?;
                let mut reader = reader.into_reader(Some(library_id), None, None, Some((self.clone(), requesting_user)), None).await?;
                let image_result = crate::tools::image_tools::reader_to_image(&mut reader.stream).await?;
                
                // Convert DynamicImage to buffer using spawn_blocking for consistency with codebase
                let buffer = tokio::task::spawn_blocking(move || {
                    use crate::tools::image_tools::save_image_native;
                    save_image_native(image_result, image::ImageFormat::Png, None, true)
                }).await
                .map_err(|e| RsError::Error(format!("Task join error: {}", e)))??;
                vec![(None, buffer)]
            },
            FileType::Video => {
                // Extract multiple video frames (existing logic)
                let percents = vec![15, 30, 45, 60, 75, 95];
                let mut video_images = Vec::new();
                for percent in percents {
                    let thumb = self.get_video_thumb(library_id, media_id, VideoTime::Percent(percent), image::ImageFormat::Png, Some(70), requesting_user).await?;
                    video_images.push((Some(percent), thumb)); // Store percent with image
                }
                // Also add media_image if it exists
                if let Ok(mut reader_response) = self.media_image(&library_id, &media_id, None, &requesting_user).await {
                    if let Ok(buffer) = convert_image_reader(reader_response.stream, image::ImageFormat::Png, None, true).await {
                        video_images.push((None, buffer)); // Store with None percent for media_image
                    }
                }
                video_images
            },
            _ => {
                // For other types, use thumbnail
                let mut reader_response = self.media_image(&library_id, &media_id, None, &requesting_user).await?;
                let buffer = convert_image_reader(reader_response.stream, image::ImageFormat::Png, None, true).await?;
                vec![(None, buffer)]
            }
        };

        let service = self.get_face_recognition_service().await?;
        let threshold = self.get_face_threshold(library_id).await?;
        let mut results = Vec::new();

        // Process faces from all images
        for (video_percent, image_buffer) in &images_data {
            let mut cursor = Cursor::new(image_buffer.clone());
            let image = crate::tools::image_tools::reader_to_image(&mut cursor).await?.image;
            let faces = service.detect_and_extract_faces_async(image).await?;

            for face in &faces {
                // Create bbox with video_percent if available
                let bbox = FaceBBox {
                    x1: face.bbox.x1,
                    y1: face.bbox.y1,
                    x2: face.bbox.x2,
                    y2: face.bbox.y2,
                    video_percent: *video_percent,
                };
                
                // Try matching with threshold
                if let Some((person_id, sim)) = self.match_face_to_person(library_id, &face.embedding, threshold).await? {
                    // High confidence → assign directly
                    let face_id = nanoid!();
                    store.add_face_embedding(
                        face_id.clone(), &person_id, face.embedding.clone(),
                        Some(media_id.to_string()), Some(bbox.clone()),
                        face.confidence, Some(face.pose), Some(sim)
                    ).await?;
                    // Note: add_face_embedding now handles media_people_mapping internally
                    results.push(DetectedFaceResult { face_id: Some(face_id), confidence: face.confidence, bbox: bbox.clone() });
                } else {
                    // No match → stage for clustering
                    let face_id = nanoid!();
                    store.add_unassigned_face(
                        face_id.clone(), face.embedding.clone(), media_id.to_string(),
                        bbox.clone(),
                        face.confidence, Some(face.pose)
                    ).await?;
                    results.push(DetectedFaceResult { face_id: Some(face_id), confidence: face.confidence, bbox: bbox.clone() });
                }
            }
        }
        
        store.mark_media_face_processed(media_id.to_string()).await?;
        Ok(results)
    }

    pub async fn cluster_unassigned_faces(&self, library_id: &str, requesting_user: &ConnectedUser) -> RsResult<ClusteringResult> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id)?;
        let unassigned = store.get_all_unassigned_faces(None, None).await?;
        
        if unassigned.len() < 3 {
            return Ok(ClusteringResult { clusters_created: 0 });
        }
        
        // Collect all face IDs before moving unassigned into closure
        let all_face_ids: Vec<String> = unassigned.iter().map(|f| f.id.clone()).collect();
        
        // Create lookup map from face_id to media_ref for threshold counting
        let face_to_media: std::collections::HashMap<String, String> = unassigned
            .iter()
            .map(|f| (f.id.clone(), f.media_ref.clone()))
            .collect();
        
        // Get threshold for this library
        let threshold = self.get_face_threshold(library_id).await?;
        let threshold_clone = threshold; // Clone for move into closure
        
        // Chinese Whispers clustering (CPU-bound)
        let clusters = tokio::task::spawn_blocking(move || {
            chinese_whispers_clustering(&unassigned, threshold_clone)
        }).await.map_err(|e| RsError::Error(format!("Clustering task failed: {}", e)))?;
        
        // Track all face IDs that were clustered (in clusters with 2+ faces)
        let mut all_clustered_face_ids = std::collections::HashSet::new();
        let mut created = 0;
        
        for (cluster_id, face_ids) in clusters {
            // Track all faces in this cluster
            for face_id in &face_ids {
                all_clustered_face_ids.insert(face_id.clone());
            }
            
            // Count unique media_refs instead of total faces
            let unique_media_count: usize = face_ids
                .iter()
                .filter_map(|face_id| face_to_media.get(face_id))
                .collect::<std::collections::HashSet<_>>()
                .len();
            
            if unique_media_count >= 10 {
                // First assign cluster_id to faces (required for promote_cluster_to_person to find them)
                store.assign_cluster_to_faces(face_ids.clone(), cluster_id.clone()).await?;
                
                // Then create person and promote
                let new_person = PersonForAdd {
                    name: format!("Unknown Person {}", nanoid!(5)),
                    generated: true,
                    ..Default::default()
                };
                let person = self.add_pesron(library_id, new_person, &ConnectedUser::ServerAdmin).await?;
                store.promote_cluster_to_person(cluster_id, person.id).await?;
                
                // Get unique media IDs from the faces in this cluster and send update events
                let cluster_media_ids: Vec<String> = face_ids
                    .iter()
                    .filter_map(|face_id| face_to_media.get(face_id))
                    .cloned()
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                self.send_media_update_events(library_id, &cluster_media_ids, requesting_user).await;
                
                created += 1;
            } else if face_ids.len() >= 2 {
                // Cluster with 2+ faces but not enough unique media to create person - assign cluster_id
                store.assign_cluster_to_faces(face_ids, cluster_id).await?;
            }
        }
        
        // Mark singleton faces as processed (they were in clustering but didn't form clusters)
        let singleton_face_ids: Vec<String> = all_face_ids.iter()
            .filter(|id| !all_clustered_face_ids.contains(*id))
            .cloned()
            .collect();
        
        if !singleton_face_ids.is_empty() {
            crate::tools::log::log_info(crate::tools::log::LogServiceType::Other, format!("Marking {} singleton faces as processed", singleton_face_ids.len()));
            store.mark_faces_as_processed(singleton_face_ids).await?;
        }
        
        Ok(ClusteringResult { clusters_created: created })
    }

    pub async fn get_unassigned_faces(&self, library_id: &str, requesting_user: &ConnectedUser) -> RsResult<Vec<UnassignedFace>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let faces = store.get_unassigned_faces().await?;
        Ok(faces)
    }

    pub async fn get_all_unassigned_faces(&self, library_id: &str, limit: Option<usize>, created_before: Option<i64>, requesting_user: &ConnectedUser) -> RsResult<Vec<UnassignedFace>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let faces = store.get_all_unassigned_faces(limit, created_before).await?;
        Ok(faces)
    }

    pub async fn get_person_faces(&self, library_id: &str, person_id: &str, requesting_user: &ConnectedUser) -> RsResult<Vec<FaceEmbedding>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let faces = store.get_person_embeddings(person_id).await?;
        Ok(faces)
    }

    pub async fn get_media_faces(&self, library_id: &str, media_id: &str, requesting_user: &ConnectedUser) -> RsResult<Vec<FaceEmbedding>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        let store = self.store.get_library_store(library_id)?;
        let faces = store.get_media_embeddings(media_id).await?;
        Ok(faces)
    }

    pub async fn delete_face(&self, library_id: &str, face_id: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id)?;
        
        // Try deleting from people_faces first
        let deleted = match store.delete_face_embedding(face_id).await {
            Ok(_) => true,
            Err(_) => {
                // If not found in people_faces, try unassigned_faces
                store.delete_unassigned_face(face_id).await?;
                true
            }
        };
        
        // Delete cached face image if it exists (ignore errors - cache cleanup is best effort)
        if deleted {
            if let Err(e) = self.remove_library_image(library_id, ".faces", face_id, &None, &None, requesting_user).await {
                log_info(crate::tools::log::LogServiceType::Other, format!("Failed to delete cached face image {}: {}", face_id, e));
            }
        }
        
        Ok(())
    }

    pub async fn assign_unassigned_face_to_person(&self, library_id: &str, face_id: &str, person_id: &str, requesting_user: &ConnectedUser) -> RsResult<()> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        
        // Get media IDs before assignment
        let media_ids = self.get_media_ids_from_face_ids(library_id, &[face_id.to_string()], true).await?;
        
        let store = self.store.get_library_store(library_id)?;
        store.assign_unassigned_face_to_person(face_id.to_string(), person_id.to_string()).await?;
        
        // Send media update events
        self.send_media_update_events(library_id, &media_ids, requesting_user).await;
        
        Ok(())
    }

    pub async fn assign_unassigned_faces_to_person(&self, library_id: &str, face_ids: &[String], person_id: &str, requesting_user: &ConnectedUser) -> RsResult<usize> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        
        // Get media IDs before assignment
        let media_ids = self.get_media_ids_from_face_ids(library_id, face_ids, true).await?;
        
        let store = self.store.get_library_store(library_id)?;
        let count = store.assign_unassigned_faces_to_person_batch(face_ids.to_vec(), person_id.to_string()).await?;
        
        // Send media update events
        self.send_media_update_events(library_id, &media_ids, requesting_user).await;
        
        Ok(count)
    }

    pub async fn unassign_faces_from_person(&self, library_id: &str, face_ids: &[String], requesting_user: &ConnectedUser) -> RsResult<usize> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        
        // Get media IDs before unassignment (faces are in people_faces)
        let media_ids = self.get_media_ids_from_face_ids(library_id, face_ids, false).await?;
        
        let store = self.store.get_library_store(library_id)?;
        let count = store.unassign_faces_from_person_batch(face_ids.to_vec()).await?;
        
        // Send media update events
        self.send_media_update_events(library_id, &media_ids, requesting_user).await;
        
        Ok(count)
    }

    pub async fn match_unassigned_faces_to_people(&self, library_id: &str, requesting_user: &ConnectedUser) -> RsResult<usize> {
        requesting_user.check_library_role(library_id, LibraryRole::Write)?;
        let store = self.store.get_library_store(library_id)?;
        let unassigned_faces = store.get_all_unassigned_faces(None, None).await?;
        
        let threshold = self.get_face_threshold(library_id).await?;
        let mut matched_count = 0;
        
        for face in unassigned_faces {
            // Skip faces with empty embeddings
            if face.embedding.is_empty() {
                continue;
            }
            
            // Try to match this face to an existing person
            match self.match_face_to_person(library_id, &face.embedding, threshold).await {
                Ok(Some((person_id, _sim))) => {
                    // Match found - assign the face to this person
                    match self.assign_unassigned_face_to_person(library_id, &face.id, &person_id, requesting_user).await {
                        Ok(_) => {
                            matched_count += 1;
                        }
                        Err(e) => {
                            crate::tools::log::log_error(
                                crate::tools::log::LogServiceType::Scheduler,
                                format!("Error assigning face {} to person {}: {:#}", face.id, person_id, e)
                            );
                        }
                    }
                }
                Ok(None) => {
                    // No match found - continue to next face
                }
                Err(e) => {
                    crate::tools::log::log_error(
                        crate::tools::log::LogServiceType::Scheduler,
                        format!("Error matching face {}: {:#}", face.id, e)
                    );
                }
            }
        }
        
        Ok(matched_count)
    }

    pub async fn get_face_image(&self, library_id: &str, face_id: &str, requesting_user: &ConnectedUser) -> RsResult<Vec<u8>> {
        requesting_user.check_library_role(library_id, LibraryRole::Read)?;
        
        // Check if cached image exists
        if self.has_library_image(library_id, ".faces", face_id, None, requesting_user).await? {
            // Load cached image
            let mut cached_image = self.library_image(library_id, ".faces", face_id, None, None, requesting_user).await?;
            let mut buffer = Vec::new();
            cached_image.stream.read_to_end(&mut buffer).await?;
            return Ok(buffer);
        }
        
        // Cache miss - generate the image
        let store = self.store.get_library_store(library_id)?;
        
        // Get face details (media_ref and bbox)
        let (media_ref, bbox) = store.get_face_by_id(face_id).await?
            .ok_or_else(|| RsError::NotFound(format!("Face not found: {}", face_id)))?;
        
        if media_ref.is_empty() {
            return Err(RsError::Error("Face has no associated media".to_string()));
        }
        
        let bbox = bbox.ok_or_else(|| RsError::Error("Face has no bounding box".to_string()))?;
        
        // Get media type
        let media = self.get_media(library_id, media_ref.clone(), requesting_user).await?
            .ok_or_else(|| RsError::NotFound(format!("Media not found: {}", media_ref)))?;
        
        // Load media image based on type
        let image_result = match media.kind {
            FileType::Photo => {
                // Load full image for photos
                let reader = self.library_file(library_id, &media_ref, None, MediaFileQuery::default(), requesting_user).await?;
                let mut reader = reader.into_reader(Some(library_id), None, None, Some((self.clone(), requesting_user)), None).await?;
                crate::tools::image_tools::reader_to_image(&mut reader.stream).await?
            },
            FileType::Video => {
                // Load video frame at the percent where face was detected, or use media_image if no percent
                match bbox.video_percent {
                    Some(percent) => {
                        let thumb_buffer = self.get_video_thumb(library_id, &media_ref, VideoTime::Percent(percent), image::ImageFormat::Png, Some(70), requesting_user).await?;
                        let mut cursor = Cursor::new(thumb_buffer);
                        crate::tools::image_tools::reader_to_image(&mut cursor).await?
                    },
                    None => {
                        // Use media_image when no video_percent is specified
                        let mut reader_response = self.media_image(library_id, &media_ref, None, requesting_user).await?;
                        crate::tools::image_tools::reader_to_image(&mut reader_response.stream).await?
                    }
                }
            },
            _ => {
                // For other types, use thumbnail
                let mut reader_response = self.media_image(library_id, &media_ref, None, requesting_user).await?;
                crate::tools::image_tools::reader_to_image(&mut reader_response.stream).await?
            }
        };
        let image = image_result.image;
        
        // Crop face using bbox with 30% padding
        // Convert FaceBBox to BBox (confidence not needed for cropping)
        let bbox_for_crop = BBox {
            x1: bbox.x1,
            y1: bbox.y1,
            x2: bbox.x2,
            y2: bbox.y2,
            confidence: 0.0, // Not used for cropping
        };
        let (cropped, _offset_x, _offset_y) = FaceRecognitionService::crop_face_with_padding(&image, &bbox_for_crop, 0.3)?;
        
        // Resize to match person image sizing and convert to AVIF format
        use crate::tools::image_tools::{ImageAndProfile, save_image_native};
        use image::{ImageFormat, GenericImageView};
        let target_size = ImageSize::Thumb.to_size(); // 258
        let (cropped_width, cropped_height) = cropped.dimensions();
        let image_bytes = tokio::task::spawn_blocking(move || {
            // Only resize if the image is larger than the target size (downscale only, no upscaling)
            let resized = if cropped_width > target_size || cropped_height > target_size {
                cropped.thumbnail(target_size, target_size)
            } else {
                cropped
            };
            let image_and_profile = ImageAndProfile {
                image: resized,
                profile: None
            };
            save_image_native(image_and_profile, ImageFormat::Avif, Some(70), false)
        }).await.map_err(|e| RsError::Error(format!("Task join error: {}", e)))??;
        
        // Cache the generated image (use ServerAdmin for write permission)
        let reader = Cursor::new(image_bytes.clone());
        if let Err(e) = self.update_library_image(library_id, ".faces", face_id, &None, &None, reader, &ConnectedUser::ServerAdmin).await {
            // Log error but don't fail the request if caching fails
            log_info(crate::tools::log::LogServiceType::Other, format!("Failed to cache face image {}: {}", face_id, e));
        }
        
        Ok(image_bytes)
    }

    /// Helper function to get media IDs from face IDs
    /// Checks both unassigned_faces and people_faces tables
    async fn get_media_ids_from_face_ids(&self, library_id: &str, face_ids: &[String], from_unassigned: bool) -> RsResult<Vec<String>> {
        let store = self.store.get_library_store(library_id)?;
        let mut media_ids = std::collections::HashSet::new();
        
        for face_id in face_ids {
            if let Ok(Some((media_ref, _))) = store.get_face_by_id(face_id).await {
                if !media_ref.is_empty() {
                    media_ids.insert(media_ref);
                }
            }
        }
        
        Ok(media_ids.into_iter().collect())
    }

    /// Helper function to send media update events for affected media
    async fn send_media_update_events(&self, library_id: &str, media_ids: &[String], requesting_user: &ConnectedUser) {
        if media_ids.is_empty() {
            return;
        }

        let store = match self.store.get_library_store(library_id) {
            Ok(s) => s,
            Err(_) => return,
        };

        let mut medias_to_send = Vec::new();
        for media_id in media_ids {
            if let Ok(Some(media)) = store.get_media(media_id, requesting_user.user_id().ok()).await {
                medias_to_send.push(MediaWithAction {
                    media,
                    action: ElementAction::Updated,
                });
            }
        }

        if !medias_to_send.is_empty() {
            self.send_media(MediasMessage {
                library: library_id.to_string(),
                medias: medias_to_send,
            });
        }
    }

    pub async fn merge_people(&self, library_id: &str, source_person_id: &str, target_person_id: &str, requesting_user: &ConnectedUser) -> RsResult<usize> {
        requesting_user.check_library_role(library_id, LibraryRole::Admin)?;
        
        // Verify both people exist
        let source_person = self.get_person(library_id, source_person_id.to_string(), requesting_user).await?
            .ok_or_else(|| RsError::NotFoundPerson(source_person_id.to_string()))?;
        let target_person = self.get_person(library_id, target_person_id.to_string(), requesting_user).await?
            .ok_or_else(|| RsError::NotFoundPerson(target_person_id.to_string()))?;
        
        // Transfer faces
        let store = self.store.get_library_store(library_id)?;
        let faces_transferred = store.transfer_faces_between_people(source_person_id, target_person_id).await?;
        
        // Delete source person (this also checks Admin permissions internally)
        self.remove_person(library_id, source_person_id, requesting_user).await?;
        
        Ok(faces_transferred)
    }

    pub async fn get_medias_for_face_processing(&self, library_id: &str, limit: usize) -> RsResult<Vec<String>> {
        let store = self.store.get_library_store(library_id)?;
        let media_ids = store.get_medias_for_face_processing(limit).await?;
        Ok(media_ids)
    }

    async fn match_face_to_person(&self, library_id: &str, embedding: &[f32], threshold: f32) -> RsResult<Option<(String, f32)>> {
        let store = self.store.get_library_store(library_id)?;
        let all_embeddings = store.get_all_embeddings().await?;
        
        let mut best_match: Option<(String, f32)> = None;
        for (person_id, person_emb) in all_embeddings {
            let sim = cosine_similarity(embedding, &person_emb);
            if sim >= threshold {
                if best_match.is_none() || sim > best_match.as_ref().unwrap().1 {
                    best_match = Some((person_id, sim));
                }
            }
        }
        Ok(best_match)
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    // Handle empty or mismatched embeddings
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    // Assuming L2-normalized embeddings: cosine = dot product
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn chinese_whispers_clustering(faces: &[UnassignedFace], threshold: f32) -> HashMap<String, Vec<String>> {
    let n = faces.len();
    crate::tools::log::log_info(crate::tools::log::LogServiceType::Other, format!("Clustering {} unassigned faces with threshold {}", n, threshold));
    
    // CHANGED: Store (neighbor_index, score) for weighted voting
    let mut adj: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    let mut similarities = Vec::new();
    
    // Build graph (O(N^2))
    for i in 0..n {
        for j in i+1..n {
            let sim = cosine_similarity(&faces[i].embedding, &faces[j].embedding);
            similarities.push(sim);
            
            if sim >= threshold {
                // CHANGED: Store the score
                adj[i].push((j, sim));
                adj[j].push((i, sim));
            }
        }
    }
    
    let edges_formed: usize = adj.iter().map(|neighbors| neighbors.len()).sum::<usize>() / 2;
    crate::tools::log::log_info(crate::tools::log::LogServiceType::Other, format!("Formed {} edges (similarity >= {})", edges_formed, threshold));
    
    if !similarities.is_empty() {
        let min_sim = similarities.iter().fold(f32::INFINITY, |a, &b| a.min(b));
        let max_sim = similarities.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        let avg_sim = similarities.iter().sum::<f32>() / similarities.len() as f32;
        crate::tools::log::log_info(crate::tools::log::LogServiceType::Other, format!("Similarity stats: min={:.3}, max={:.3}, avg={:.3}", min_sim, max_sim, avg_sim));
    }

    // Initialize labels (each node is its own class initially)
    let mut labels: Vec<usize> = (0..n).collect();
    
    // Iterations
    let iterations = 20;
    let mut rng = rand::thread_rng(); // Initialize RNG once

    for iter in 0..iterations {
        let mut changes = 0;
        let mut indices: Vec<usize> = (0..n).collect();
        
        // CHANGED: Shuffle is critical for CW stability
        indices.shuffle(&mut rng);

        for &i in &indices {
            // Find best label among neighbors using Weighted Voting
            // Key: Label, Value: Sum of similarity scores
            let mut label_scores: HashMap<usize, f32> = HashMap::new();
            
            // CHANGED: Weighted voting logic
            if adj[i].is_empty() {
                continue; // No neighbors, keep original label
            }

            for &(neighbor_idx, score) in &adj[i] {
                let neighbor_label = labels[neighbor_idx];
                *label_scores.entry(neighbor_label).or_insert(0.0) += score;
            }
            
            // Find label with highest total score
            // Uses partial_cmp for floats
            if let Some((&best_label, _)) = label_scores.iter()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal)) 
            {
                if labels[i] != best_label {
                    labels[i] = best_label;
                    changes += 1;
                }
            }
        }
        
        if changes == 0 {
            crate::tools::log::log_info(crate::tools::log::LogServiceType::Other, format!("Clustering converged after {} iterations", iter + 1));
            break;
        }
    }

    // Group by label
    let mut clusters = HashMap::new();
    for (i, &label) in labels.iter().enumerate() {
        // Use consistent naming (e.g. cluster_0, cluster_5)
        // Note: The 'label' is just an arbitrary index from the initial set
        let cluster_id = format!("cluster_{}", label);
        clusters.entry(cluster_id).or_insert_with(Vec::new).push(faces[i].id.clone());
    }
    
    let cluster_sizes: Vec<usize> = clusters.values().map(|v| v.len()).collect();
    crate::tools::log::log_info(crate::tools::log::LogServiceType::Other, format!("Created {} raw clusters (sizes: {:?})", clusters.len(), cluster_sizes));
    
    // Filter out singleton clusters (only keep clusters with 2+ faces)
    clusters.retain(|_, face_ids| face_ids.len() >= 2);
    
    crate::tools::log::log_info(crate::tools::log::LogServiceType::Other, format!("After filtering singletons: {} clusters remaining", clusters.len()));
    
    clusters
}
