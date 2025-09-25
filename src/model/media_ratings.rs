use rs_plugin_common_interfaces::ElementType;
use serde::{Deserialize, Serialize};

use crate::{domain::{deleted::RsDeleted, episode::Episode, library::LibraryRole, media_rating::RsMediaRating}, error::{RsError, RsResult}, Error};

use super::{store::sql::SqlOrder, users::ConnectedUser, ModelController};



#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaRatingsQuery {
    pub after: Option<i64>,
    pub media: Option<String>,
    pub min_rating: Option<f64>,
    pub max_rating: Option<f64>,
    #[serde(default)]
    pub order: SqlOrder,
}


#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
pub struct MediasRatingMessage {
    pub library: String,
    pub rating: RsMediaRating
}


impl ModelController {

    pub async fn send_media_rating(&self, message: MediasRatingMessage) {
        let mapping: Vec<crate::domain::library::UserMapping> = self.get_library_mapped_users(&message.library).await.ok().unwrap_or_default();

		self.for_connected_users(&message, |user, socket, message| {

			if let Ok(user_id) = user.user_id() {
                if user_id == message.rating.user_ref {
				    let _ = socket.emit("media_rating", message);
                }
			}
		});

        for map in mapping.clone() {
            if message.rating.user_ref == map.to {
                let mut message = message.clone();
                message.rating.user_ref = map.from.clone();
                self.for_connected_users(&message, |user, socket, message| {

                    if let Ok(user_id) = user.user_id() {
                        if user_id == message.rating.user_ref {
                            let _ = socket.emit("media_rating", message);
                        }
                    }
                });
            }

            if message.rating.user_ref == map.from {
                let mut message = message.clone();
                message.rating.user_ref = map.to;
                self.for_connected_users(&message, |user, socket, message| {

                    if let Ok(user_id) = user.user_id() {
                        if user_id == message.rating.user_ref {
                            let _ = socket.emit("media_rating", message);
                        }
                    }
                });
            }
        }
	}


	pub async fn get_medias_ratings(&self, library_id: &str, query: MediaRatingsQuery, requesting_user: &ConnectedUser) -> RsResult<Vec<RsMediaRating>> {
        let original_user_id = requesting_user.user_id()?;
        let mut user_id = self.get_library_mapped_user(library_id, original_user_id.clone()).await?;

        let store = self.store.get_library_store(library_id)?;
        let mut ratings = store.get_medias_ratings(query, user_id.clone()).await?;

        if user_id != original_user_id {
            for rating in ratings.iter_mut() {
                if rating.user_ref == user_id {
                    rating.user_ref = original_user_id.clone();
                }     
            }
        }

		Ok(ratings)
	}    
    
    pub async fn get_media_rating(&self, library_id: &str, media_ref: String, requesting_user: &ConnectedUser) -> RsResult<RsMediaRating> {
        let rating = self.get_medias_ratings(library_id, MediaRatingsQuery { media: Some(media_ref.clone()), ..Default::default() }, requesting_user).await?;
        let p = rating.into_iter().next().ok_or(RsError::NotFound(format!("Media rating not found: {} for user {:?}", media_ref, requesting_user)))?;
		Ok(p)
	}

    pub async fn set_media_rating(&self, library_id: &str, media_ref: String, rating: f64, requesting_user: &ConnectedUser) -> RsResult<RsMediaRating> {
        let mut user_id = self.get_library_mapped_user(library_id, requesting_user.user_id()?).await?;

        let store = self.store.get_library_store(library_id)?;
        store.set_media_rating(media_ref.clone(), user_id, rating).await?;
        
        let rating = self.get_media_rating(library_id, media_ref, requesting_user).await?;

        let message = MediasRatingMessage {
            library: library_id.to_string(),
            rating: rating.clone()
        };
        self.send_media_rating(message).await;
		Ok(rating)
	}
}