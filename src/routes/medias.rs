
use std::{path::PathBuf, str::FromStr};

use crate::{domain::{media::{GroupMediaDownload, MediaDownloadUrl, MediaForUpdate, MediaItemReference, MediaWithAction, MediasMessage}, ElementAction}, model::{medias::{MediaFileQuery, MediaQuery}, series::{SerieForUpdate, SerieQuery}, users::ConnectedUser, ModelController}, plugins::sources::SourceRead, tools::prediction::predict_net, Error, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use futures::TryStreamExt;
use hyper::{header::ACCEPT_RANGES, StatusCode};
use rs_plugin_common_interfaces::request::RsRequest;
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
		.route("/", post(handler_post))
		.route("/", patch(handler_multi_patch))
		.route("/exist", get(handler_list))
		.route("/download", post(handler_download))
		.route("/request", post(handler_add_request))
		.route("/:id/metadata", get(handler_get))
		.route("/:id/metadata/refresh", get(handler_refresh))
		.route("/:id/sharetoken", get(handler_sharetoken))
		.route("/:id/predict", get(handler_predict))
		.route("/:id", get(handler_get_file))
		.route("/:id/backup/metadatas", get(handler_get_backup_medata))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))
		.route("/:id/image", get(handler_image))
		.route("/:id/image", post(handler_post_image))
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
		let libraries = mc.count_medias(&library_id, old_query, &user).await?;
		let body = Json(json!(libraries));
		Ok(body)
	} else {
		let libraries = mc.count_medias(&library_id, query, &user).await?;
		let body = Json(json!(libraries));
		Ok(body)
	}

}

#[derive(Debug, Serialize, Deserialize)]
struct ExistQuery {
	pub hash: String
}


async fn handler_exist(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ExistQuery>) -> Result<Json<Value>> {
	
	let libraries = mc.get_media_by_hash(&library_id, query.hash, &user).await?;
	let body = Json(json!(libraries));
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

async fn handler_get(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_media(&library_id, media_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_refresh(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	mc.process_media(&library_id, &media_id, &user).await?;
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
	let prediction = mc.prediction(&library_id, &media_id, query.tag, &user).await?;
	let body = Json(json!(prediction));
	//println!("BODY {:?}", body);
	Ok(body)
}

async fn handler_get_file(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, range: Option<RangeDefinition>, Query(query): Query<MediaFileQuery>) -> Result<Response> {
	let reader = mc.library_file(&library_id, &media_id, range.clone(), query, &user).await?;
	Ok(reader.into_response(&library_id, range, None, Some((mc.clone(), &user))).await?)
}

async fn handler_get_backup_medata(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let reader = mc.get_backup_file(&library_id, &media_id, &user).await?;
	Ok(Json(json!(reader)))
}

async fn handler_patch(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<MediaForUpdate>) -> Result<Json<Value>> {
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
			info = serde_json::from_str(&field.text().await?)?;
		} else if name == "file" {
			let filename = field.file_name().unwrap().to_string();
			let mime = field.content_type().map(|c| c.to_owned());
			info.name = info.name.or(Some(filename.clone()));
			info.mimetype = mime;
			let reader = StreamReader::new(field.map_err(|multipart_error| {
				std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
			}));
			let media = mc.add_library_file(&library_id, &filename, Some(info), reader, &user).await?;
			return Ok(Json(json!(media)))
		}
    }
	Ok(Json(json!({"message": "No media found"})))
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
        //let name = field.name().unwrap().to_string();
		//let filename = field.file_name().unwrap().to_string();
		//let mime: String = field.content_type().unwrap().to_string();
        //let data = field.bytes().await.unwrap();

		let reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		
        //println!("Length of `{}` {}  {} is {} bytes", name, filename, mime, data.len());
			mc.update_media_image(&library_id, &media_id, reader, &user).await?;
    }
	
    Ok(Json(json!({"data": "ok"})))
}