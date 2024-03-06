
use crate::{domain::media::GroupMediaDownload, model::{medias::MediaQuery, series::{SerieForAdd, SerieForUpdate, SerieQuery}, users::ConnectedUser, ModelController}, tools::prediction::predict, Error, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use futures::TryStreamExt;
use hyper::{header::ACCEPT_RANGES, StatusCode};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::io::{ReaderStream, StreamReader};

use super::{mw_range::RangeDefinition, ImageRequestOptions, ImageUploadOptions};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/", post(handler_post))
		.route("/:id/metadata", get(handler_get))
		.route("/:id/predict", get(handler_predict))
		.route("/:id", get(handler_get_file))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))
		.route("/:id/image", get(handler_image))
		.route("/:id/image", post(handler_post_image))
		.with_state(mc.clone())
		.nest("/:id/", super::episodes::routes(mc))
        
}

async fn handler_list(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<MediaQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_medias(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_get(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_media(&library_id, media_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_predict(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let mut reader_response = mc.media_image(&library_id, &media_id, None, &user).await?;

	let mut buffer = Vec::new();
    reader_response.stream.read_to_end(&mut buffer).await?;
	let prediction = predict(buffer)?;
	let body = Json(json!(prediction));
	Ok(body)
}

async fn handler_get_file(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, range: Option<RangeDefinition>) -> Result<Response> {
	let reader_response = mc.library_file(&library_id, &media_id, range.clone(), &user).await?;
	let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);
	//println!("range req: {:?}", range);
	let status = if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
    Ok((status, headers, body).into_response())
}

async fn handler_patch(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<SerieForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_serie(&library_id, media_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_serie(&library_id, &media_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, mut multipart: Multipart) -> Result<Json<Value>> {

	while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_string();
		println!("name: {} ",name);
		if name == "info" {
			let info: GroupMediaDownload = serde_json::from_str(&field.text().await?)?;
			println!("INFO: {:?}", info);
		}
		//let filename = field.file_name().unwrap().to_string();
		//let mime: String = field.content_type().unwrap().to_string();
        //let data = field.bytes().await.unwrap();

		/*let reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		
        //println!("Length of `{}` {}  {} is {} bytes", name, filename, mime, data.len());
			mc.update_serie_image(&library_id, &media_id, &query.kind, reader, &user).await?;*/
    }
	Ok(Json(json!({"data": "ok"})))
}


async fn handler_image(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Response> {
	let reader_response = mc.media_image(&library_id, &media_id, query.size, &user).await?;

	let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);
	
    Ok((headers, body).into_response())
}

#[debug_handler]
async fn handler_post_image(Path((library_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageUploadOptions>, mut multipart: Multipart) -> Result<Json<Value>> {
	while let Some(field) = multipart.next_field().await.unwrap() {
        //let name = field.name().unwrap().to_string();
		//let filename = field.file_name().unwrap().to_string();
		//let mime: String = field.content_type().unwrap().to_string();
        //let data = field.bytes().await.unwrap();

		let reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		
        //println!("Length of `{}` {}  {} is {} bytes", name, filename, mime, data.len());
			mc.update_serie_image(&library_id, &media_id, &query.kind, reader, &user).await?;
    }
	
    Ok(Json(json!({"data": "ok"})))
}