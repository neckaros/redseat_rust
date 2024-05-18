
use crate::{domain::serie::Serie, model::{episodes::EpisodeQuery, series::{SerieForUpdate, SerieQuery}, users::ConnectedUser, ModelController}, tools::image_tools::ImageType, Error, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post, put}, Json, Router};
use futures::TryStreamExt;
use rs_plugin_common_interfaces::lookup::RsLookupMovie;
use serde_json::{json, Value};
use tokio::io::AsyncRead;
use tokio_util::io::{ReaderStream, StreamReader};

use super::{ImageRequestOptions, ImageUploadOptions};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/trending", get(handler_trending))
		.route("/ondeck", get(handler_ondeck))
		.route("/upcoming", get(handler_upcoming))
		.route("/episodes", get(handler_list_episodes))
		.route("/search", get(handler_seach_series))
		.route("/", post(handler_post))
		.route("/:id", get(handler_get))
		.route("/:id", patch(handler_patch))
		.route("/:id/refresh", get(handler_refresh))
		.route("/:id/import", put(handler_import))
		.route("/:id", delete(handler_delete))
		.route("/:id/image", get(handler_image))
		.route("/:id/image", post(handler_post_image))
		.with_state(mc.clone())
		.nest("/:id/", super::episodes::routes(mc))
        
}

async fn handler_list(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<SerieQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_series(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_list_episodes(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<EpisodeQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_episodes(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_seach_series(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<RsLookupMovie>) -> Result<Json<Value>> {
	let libraries = mc.search_serie(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_trending(State(mc): State<ModelController>) -> Result<Json<Value>> {
	let libraries = mc.trending_shows().await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_upcoming(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<EpisodeQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_episodes_upcoming(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_ondeck(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<EpisodeQuery>) -> Result<Json<Value>> {
	let episodes = mc.get_episodes_ondeck(&library_id, query, &user).await?;
	let body = Json(json!(episodes));
	Ok(body)
}

async fn handler_get(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_serie(&library_id, serie_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_refresh(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.refresh_serie(&library_id, &serie_id, &user).await?;
	mc.refresh_episodes(&library_id, &serie_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}


async fn handler_import(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.import_serie(&library_id, &serie_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_patch(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<SerieForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_serie(&library_id, serie_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_serie(&library_id, &serie_id, false, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(serie): Json<Serie>) -> Result<Json<Value>> {
	let created_serie = mc.add_serie(&library_id, serie, &user).await?;
	let body = Json(json!(created_serie));
	Ok(body)
}


async fn handler_image(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Response> {
	let reader_response = mc.serie_image(&library_id, &serie_id, query.kind.clone(), query.size.clone(), &user).await;


	if let Ok(reader_response) =reader_response {
		let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
		let stream = ReaderStream::new(reader_response.stream);
		let body = Body::from_stream(stream);
		
		Ok((headers, body).into_response())
	} else if query.kind.as_ref().unwrap_or(&ImageType::Poster) == &ImageType::Card {
		let reader_response = mc.serie_image(&library_id, &serie_id, Some(ImageType::Background), query.size, &user).await?;
		let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
		let stream = ReaderStream::new(reader_response.stream);
		let body = Body::from_stream(stream);
		
		Ok((headers, body).into_response())
	} else {
		Err(Error::NotFound)
	}
}

#[debug_handler]
async fn handler_post_image(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageUploadOptions>, mut multipart: Multipart) -> Result<Json<Value>> {
	while let Some(field) = multipart.next_field().await.unwrap() {
        //let name = field.name().unwrap().to_string();
		//let filename = field.file_name().unwrap().to_string();
		//let mime: String = field.content_type().unwrap().to_string();
        //let data = field.bytes().await.unwrap();

		let reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		
        //println!("Length of `{}` {}  {} is {} bytes", name, filename, mime, data.len());
			mc.update_serie_image(&library_id, &serie_id, &query.kind, reader, &user).await?;
    }
	
    Ok(Json(json!({"data": "ok"})))
}