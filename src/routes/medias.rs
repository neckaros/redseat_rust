
use std::{io::Cursor, path::PathBuf, str::FromStr};

use crate::{domain::{media::{self, GroupMediaDownload, MediaDownloadUrl, MediaForUpdate, MediaItemReference, MediaWithAction, MediasMessage}, ElementAction}, error::RsError, model::{self, medias::{MediaFileQuery, MediaQuery}, series::{SerieForUpdate, SerieQuery}, users::ConnectedUser, ModelController}, plugins::sources::{error::SourcesError, SourceRead}, tools::{log::{log_error, log_info}, prediction::predict_net}, Error, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use futures::TryStreamExt;
use hyper::{header::ACCEPT_RANGES, StatusCode};
use rs_plugin_common_interfaces::{request::RsRequest, video::VideoConvertRequest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::io::{ReaderStream, StreamReader};
use axum_extra::extract::Query;
use super::{mw_range::RangeDefinition, ImageRequestOptions, ImageUploadOptions};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/count", get(handler_count))
		.route("/loc", get(handler_locs))
		.route("/", delete(handler_multi_delete))
		.route("/", post(handler_post))
		.route("/", patch(handler_multi_patch))
		.route("/exist", get(handler_exist))
		.route("/download", post(handler_download))
		.route("/request", post(handler_add_request))
		.route("/transfert/:destination", post(handler_transfert))
		.route("/:id/split", get(handler_split))
		.route("/:id/metadata", get(handler_get))
		.route("/:id/metadata/refresh", get(handler_refresh))
		.route("/:id/sharetoken", get(handler_sharetoken))
		.route("/:id/predict", get(handler_predict))
		.route("/:id/convert", post(handler_convert))
		.route("/:id/convert/plugin/:plugin_id", post(handler_convert_plugin))
		.route("/:id", get(handler_get_file))
		.route("/:id/backup/last", get(handler_get_last_backup))

		.route("/:id/backup/:backupid", get(handler_get_backup))
		.route("/:id/backup/metadatas", get(handler_get_backup_medata))
		.route("/:id", patch(handler_patch))
		.route("/:id/progress", patch(handler_patch_progress))
		.route("/:id/rating", patch(handler_patch_rating))
		.route("/:id", delete(handler_delete))
		.route("/:id/image", get(handler_image))
		.route("/:id/image", post(handler_post_image))
		.route("/:id/faces", get(handler_get_media_faces))
		.with_state(mc.clone())
		.nest("/:id/", super::episodes::routes(mc))
        
}

async fn handler_list(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<MediaQuery>) -> Result<Json<Value>> {
	if let Some(filter) = &query.filter {
		let old_query = serde_json::from_str::<MediaQuery>(filter)?;
		//old_query.page_key = query.page_key;
		let libraries = mc.get_medias(&library_id, old_query, &user).await?;
		let body = Json(json!(libraries));
		Ok(body)
	} else {
		let libraries = mc.get_medias(&library_id, query, &user).await?;
		let body = Json(json!(libraries));
		Ok(body)
	}
}

async fn handler_count(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<MediaQuery>) -> Result<Json<Value>> {
	if let Some(filter) = &query.filter {
		let old_query = serde_json::from_str::<MediaQuery>(filter)?;
		//old_query.page_key = query.page_key;
		let count = mc.count_medias(&library_id, old_query, &user).await?;
		let body = Json(json!({"count": count}));
		Ok(body)
	} else {
		let count = mc.count_medias(&library_id, query, &user).await?;
		let body = Json(json!({"count": count}));
		Ok(body)
	}

}

#[derive(Debug, Serialize, Deserialize)]
struct ExistQuery {
	pub hash: String
}


async fn handler_exist(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ExistQuery>) -> Result<Json<Value>> {
	
	let media = mc.get_media_by_hash(&library_id, query.hash, true, &user).await?;
	
	let body = Json(json!({"exist": media.is_some(), "media": media}));
	Ok(body)
	

}


#[derive(Debug, Serialize, Deserialize)]
struct LocQuery {
	pub precision: Option<u32>
}

async fn handler_locs(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<LocQuery>) -> Result<Json<Value>> {

	let libraries = mc.get_locs(&library_id, query.precision, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)

}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")] 
struct MediasTransfertRequest {
	ids: Vec<String>,
	#[serde(default)]
	delete_original: bool
}

async fn handler_transfert(Path((library_id, destination)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(query): Json<MediasTransfertRequest>) -> Result<Json<Value>> {
	let mut new_medias = vec![];
	for id in query.ids {
		let existing = mc.get_media(&library_id, id.clone(), &user).await?.ok_or(SourcesError::UnableToFindMedia(library_id.to_string(), id.to_string(), "handler_transfert".to_string()))?;
		let reader = mc.library_file(&library_id, &id, None, MediaFileQuery { raw: true, ..Default::default() }, &user).await?.into_reader(Some(&library_id), None, None, Some((mc.clone(), &user)), None).await?;
		let media = mc.add_library_file(&destination, &existing.name, Some(existing.clone().into()), reader.stream, &user).await?;
		new_medias.push(media)
	}
	let body = Json(json!(new_medias));
	Ok(body)
}


#[derive(Debug, Serialize, Deserialize)]
struct SplitQuery {
	pub from: u32,
	pub to: u32
}
async fn handler_split(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<SplitQuery>) -> Result<Json<Value>> {
	let media = mc.split_media(&library_id, media_id, query.from, query.to, &user).await?;
	let body = Json(json!(media));
	Ok(body)
}


async fn handler_get(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let mut library = mc.get_media(&library_id, media_id.clone(), &user).await?;
	if let Some(ref mut media) = library {
		let faces = mc.get_media_faces(&library_id, &media_id, &user).await?;
		media.faces = Some(faces);
	}
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_refresh(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	mc.update_file_infos(&library_id, &media_id, &user, true).await?;
	mc.process_media(&library_id, &media_id, false, true, &user).await?;
	let media = mc.get_media(&library_id, media_id, &user).await?;
	let body = Json(json!(media));
	Ok(body)
}


async fn handler_sharetoken(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<String> {
	let sharetoken = mc.get_file_share_token(&library_id, &media_id, 6 * 60 * 60,  &user).await?;
	Ok(sharetoken)
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct PredictOption {
	#[serde(default)]
	pub tag: bool
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct UploadOption {
	#[serde(default)]
	pub spawn: bool
}

async fn handler_predict(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<PredictOption>) -> Result<Json<Value>> {
	let prediction = mc.prediction(&library_id, &media_id, query.tag, &user, true).await?;
	let body = Json(json!(prediction));
	//println!("BODY {:?}", body);
	Ok(body)
}



async fn handler_convert(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(query): Json<VideoConvertRequest>) -> Result<Json<Value>> {

	mc.convert(&library_id, &media_id, query.clone(), None, &user).await?;

	Ok(Json(json!(query)))
	
}
async fn handler_convert_plugin(Path((library_id, media_id, plugin_id)): Path<(String, String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(query): Json<VideoConvertRequest>) -> Result<Json<Value>> {

	mc.convert(&library_id, &media_id, query.clone(), Some(plugin_id), &user).await?;

	Ok(Json(json!(query)))
	
}
async fn handler_get_file(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, range: Option<RangeDefinition>, Query(query): Query<MediaFileQuery>) -> Result<Response> {
	let reader = mc.library_file(&library_id, &media_id, range.clone(), query, &user).await?;
	reader.into_response(&library_id, range, None, Some((mc.clone(), &user))).await
}

async fn handler_get_last_backup(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Response<Body>> {
	let reader = mc.get_backup_media(&library_id, &media_id, None, &user).await?;
	let response = reader.into_response(&library_id, None, None, Some((mc.clone(), &user))).await?;
	Ok(response)
}


async fn handler_get_backup(Path((library_id, media_id, backup_file_id)): Path<(String, String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Response<Body>> {
	let reader = mc.get_backup_media(&library_id, &media_id, Some(&backup_file_id), &user).await?;
	let response = reader.into_response(&library_id, None, None, Some((mc.clone(), &user))).await?;
	Ok(response)
}


async fn handler_get_backup_medata(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let reader = mc.get_library_media_backup_files(&library_id, &media_id, &user).await?;
	Ok(Json(json!(reader)))
}

async fn handler_patch(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<MediaForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_media(&library_id, media_id, update, true, &user).await?;
	Ok(Json(json!(new_credential)))
}


#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct MediaProgressUpdateQuery {
	#[serde(default)]
	pub progress: u64
}

async fn handler_patch_progress(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, body: Json<MediaProgressUpdateQuery>) -> Result<Json<Value>> {
	let new_credential = mc.set_media_progress(&library_id, media_id, body.progress, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_patch_rating(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<MediaForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_media(&library_id, media_id, update, true, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_media(&library_id, &media_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, mut multipart: Multipart) -> Result<Json<Value>> {
	let mut info:MediaForUpdate = MediaForUpdate::default();
	while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_string();
		if name == "info" {
			let text = &field.text().await?;
			info = serde_json::from_str(&text)?;
		} else if name == "file" {
			let filename = field.file_name().unwrap().to_string();
			let mime = if info.mimetype.is_none() { field.content_type().map(|c| c.to_owned()) } else { info.mimetype };
			let size = field.headers().get("Content-Length")
            .and_then(|len| len.to_str().ok())
            .and_then(|len_str| len_str.parse::<u64>().ok());
        	
			println!("Expected file length: {:?}", size);

			info.name = info.name.or(Some(filename.clone()));
			info.mimetype = mime;
			info.size = size;
			
			let reader = StreamReader::new(field.map_err(|multipart_error| {
				std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
			}));


			
			let media = mc.add_library_file(&library_id, &filename, Some(info), reader, &user).await?;
			return Ok(Json(json!(media)))
		}
    }
	Err(Error::Error("No media provided".to_owned()))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct MediasRemoveRequest {
	ids: Vec<String>,
}

async fn handler_multi_delete(Path(library_id): Path<String>, State(mc): State<ModelController>, requesting_user: ConnectedUser, Json(updates): Json<MediasRemoveRequest>) -> Result<Json<Value>> {
	let mut removed = vec![];
	for id in updates.ids {
		let removed_media = mc.remove_media(&library_id, &id, &requesting_user).await;
		if let Err(RsError::Model(model::error::Error::MediaNotFound(media_id))) = removed_media {
			log_info(crate::tools::log::LogServiceType::Other, format!("Media id {} not delete (not found)", media_id));
		} else if let Err(error) = removed_media {
			log_error(crate::tools::log::LogServiceType::Other, format!("Media id {} not delete (error: {:?})", id, error));
		} else if let Ok(media) = removed_media {
			removed.push(media);
		}
        
    }
	mc.send_media(MediasMessage { library: library_id.to_string(), medias: removed.iter().map(|m| MediaWithAction { media: m.clone(), action: ElementAction::Deleted}).collect()});
	Ok(Json(json!(removed)))
}
#[derive(Debug, Serialize, Deserialize, Clone)]
struct MediasUpdateRequest {
	ids: Vec<String>,
	update: MediaForUpdate
}


async fn handler_multi_patch(Path(library_id): Path<String>, State(mc): State<ModelController>, requesting_user: ConnectedUser, Json(updates): Json<MediasUpdateRequest>) -> Result<Json<Value>> {
	let mut updated = vec![];
	for id in updates.ids {
        updated.push(mc.update_media(&library_id, id, updates.update.clone(), true, &requesting_user).await?);
    }
	mc.send_media(MediasMessage { library: library_id.to_string(), medias: updated.iter().map(|m| MediaWithAction { media: m.clone(), action: ElementAction::Updated}).collect()});

	Ok(Json(json!(updated)))
}

async fn handler_download(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<UploadOption>, Json(download): Json<GroupMediaDownload<MediaDownloadUrl>>) -> Result<Json<Value>> {

	if query.spawn {
		tokio::spawn(async move {
			let _ = mc.download_library_url(&library_id,  download, &user).await.expect("Unable to download");
		});
		
		Ok(Json(json!({"downloading": true})))
	} else {
		let body = mc.download_library_url(&library_id,  download, &user).await?;
		Ok(Json(json!(body)))
	}
}


async fn handler_add_request(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(request): Json<RsRequest>) -> Result<Json<Value>> {
	
	let added = mc.medias_add_request(&library_id,  request, None, &user).await.expect("Unable to download");

	
	Ok(Json(json!(added)))
}

async fn handler_image(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Response> {
	let reader_response = mc.media_image(&library_id, &media_id, query.size, &user).await?;

	let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);
	
    Ok((headers, body).into_response())
}

#[debug_handler]
async fn handler_post_image(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, mut multipart: Multipart) -> Result<Json<Value>> {
	while let Some(field) = multipart.next_field().await.unwrap() {
		let mut reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		// Read all bytes from the field into a buffer
		let mut data = Vec::new();
		tokio::io::copy(&mut reader, &mut data).await?;
		let reader = Box::pin(Cursor::new(data));
		mc.update_media_image(&library_id, &media_id, Box::pin(reader), &user).await?;
    }
	
    Ok(Json(json!({"data": "ok"})))
}

async fn handler_get_media_faces(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser
) -> Result<Json<Value>> {
    let faces = mc.get_media_faces(&library_id, &media_id, &user).await?;
    Ok(Json(json!(faces)))
}