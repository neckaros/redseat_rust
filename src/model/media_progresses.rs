use rs_plugin_common_interfaces::ElementType;
use serde::{Deserialize, Serialize};

use crate::{domain::{deleted::RsDeleted, episode::Episode, library::LibraryRole, media_progress::RsMediaProgress, media_rating::RsMediaRating, progress}, error::{RsError, RsResult}, routes::sse::SseEvent, Error};

use super::{store::sql::SqlOrder, users::ConnectedUser, ModelController};



#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaProgressesQuery {
    pub after: Option<i64>,
    pub media: Option<String>,
    #[serde(default)]
    pub order: SqlOrder,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MediasProgressMessage {
    pub library: String,
    pub progress: RsMediaProgress
}



impl ModelController {

	pub async fn send_media_progress(&self, message: MediasProgressMessage) {
        let mapping: Vec<crate::domain::library::UserMapping> = self.get_library_mapped_users(&message.library).await.ok().unwrap_or_default();

        // Broadcast via SSE
        self.broadcast_sse(SseEvent::MediaProgress(message.clone()));

		self.for_connected_users(&message, |user, socket, message| {

			if let Ok(user_id) = user.user_id() {
                if user_id == message.progress.user_ref {
				    let _ = socket.emit("media_progress", message);
                }
			}
		});

        for map in mapping.clone() {
            if message.progress.user_ref == map.to {
                let mut message = message.clone();
                message.progress.user_ref = map.from.clone();
                // Broadcast mapped SSE event
                self.broadcast_sse(SseEvent::MediaProgress(message.clone()));
                self.for_connected_users(&message, |user, socket, message| {

                    if let Ok(user_id) = user.user_id() {
                        if user_id == message.progress.user_ref {
                            let _ = socket.emit("media_progress", message);
                        }
                    }
                });
            }
            if message.progress.user_ref == map.from {
                let mut message = message.clone();
                message.progress.user_ref = map.to;
                // Broadcast mapped SSE event
                self.broadcast_sse(SseEvent::MediaProgress(message.clone()));
                self.for_connected_users(&message, |user, socket, message| {

                    if let Ok(user_id) = user.user_id() {
                        if user_id == message.progress.user_ref {
                            let _ = socket.emit("media_progress", message);
                        }
                    }
                });
            }
        }
	}

	pub async fn get_medias_progresses(&self, library_id: &str, query: MediaProgressesQuery, requesting_user: &ConnectedUser) -> RsResult<Vec<RsMediaProgress>> {
        let original_user_id = requesting_user.user_id()?;
        let mut user_id = self.get_library_mapped_user(library_id, original_user_id.clone()).await?;
        let store = self.store.get_library_store(library_id)?;
        let mut progresses = store.get_medias_progresses(query, user_id.clone()).await?;
        if user_id != original_user_id {
            for progress in progresses.iter_mut() {
                if progress.user_ref == user_id {
                    progress.user_ref = original_user_id.clone();
                }     
            }
        }
		Ok(progresses)
	}

    pub async fn get_media_progress(&self, library_id: &str, media_ref: String, requesting_user: &ConnectedUser) -> RsResult<RsMediaProgress> {
        let progress = self.get_medias_progresses(library_id, MediaProgressesQuery { media: Some(media_ref.clone()), ..Default::default() }, requesting_user).await?;
        let p = progress.into_iter().next().ok_or(RsError::NotFound(format!("Media progress not found: {} for user {:?}", media_ref, requesting_user)))?;
		Ok(p)
	}

    pub async fn set_media_progress(&self, library_id: &str, media_ref: String, progress: u64, requesting_user: &ConnectedUser) -> RsResult<RsMediaProgress> {
        let mut user_id = self.get_library_mapped_user(library_id, requesting_user.user_id()?).await?;
        let store = self.store.get_library_store(library_id)?;
        store.set_media_progress(media_ref.clone(), user_id, progress).await?;
        let progress = self.get_media_progress(library_id, media_ref, requesting_user).await?;

        let message = MediasProgressMessage {
            library: library_id.to_string(),
            progress: progress.clone()
        };
        self.send_media_progress(message).await;
		Ok(progress)
	}
}