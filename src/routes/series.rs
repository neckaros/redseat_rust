
use std::{convert::Infallible, io::Cursor, time::Duration};

use crate::{domain::serie::Serie, error::RsError, model::{episodes::EpisodeQuery, series::{SerieForUpdate, SerieQuery}, users::ConnectedUser, ModelController}, plugins::sources::error::SourcesError, Error, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{sse::{Event, KeepAlive, Sse}, IntoResponse, Response}, routing::{delete, get, patch, post, put}, Json, Router};
use futures::{Stream, TryStreamExt};
use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, lookup::RsLookupMovie, ExternalImage, ImageType};
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
		.route("/searchstream", get(handler_search_series_stream))
		.route("/", post(handler_post))
		.route("/:id", get(handler_get))
		.route("/:id", patch(handler_patch))
		.route("/:id/refresh", get(handler_refresh))
		.route("/:id/import", put(handler_import))
		.route("/:id", delete(handler_delete))
		.route("/:id/image", get(handler_image))
		.route("/:id/image", post(handler_post_image))
		.route("/:id/image/search", get(handler_image_search))
		.route("/:id/image/fetch", post(handler_image_fetch))
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

async fn handler_search_series_stream(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<RsLookupMovie>) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
	let mut rx = mc.search_serie_stream(&library_id, query, &user).await?;

	let stream = async_stream::stream! {
		while let Some((name, batch)) = rx.recv().await {
			if let Ok(data) = serde_json::to_string(&json!({ &name: batch })) {
				yield Ok(Event::default().event("results").data(data));
			}
		}
	};

	Ok(Sse::new(stream).keep_alive(
		KeepAlive::new()
			.interval(Duration::from_secs(30))
			.text("ping"),
	))
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
	} else if query.defaulting { 
		if query.kind.as_ref().unwrap_or(&ImageType::Poster) == &ImageType::Card {
			let reader_response = mc.serie_image(&library_id, &serie_id, Some(ImageType::Background), query.size, &user).await?;
			let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
			let stream = ReaderStream::new(reader_response.stream);
			let body = Body::from_stream(stream);
			
			Ok((headers, body).into_response())
		} else if let Err(err) = reader_response {
			Err(Error::NotFound(format!("Unable to find serie image with defaulting: {} {} {:?}", library_id, serie_id, err)))
		} else {
			Err(Error::NotFound(format!("Unable to find serie image with defaulting: {} {}", library_id, serie_id)))
		}
	} else {
		if let Err(err) = reader_response {
			Err(Error::NotFound(format!("Unable to find serie image without defaulting: {} {} {:?}", library_id, serie_id, err)))
		} else {
			Err(Error::NotFound(format!("Unable to find serie image without defaulting: {} {}", library_id, serie_id)))
		}
	}
}

async fn handler_image_search(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Json<Value>> {
	let serie = mc.get_serie(&library_id, serie_id.clone(), &user).await?.ok_or(SourcesError::UnableToFindSerie(library_id, serie_id, "handler_image_search".to_string()))?;
	let ids: RsIds = serie.into();
	let result = mc.get_serie_images(&ids).await?;

	Ok(Json(json!(result)))
}





async fn handler_image_fetch(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(externalImage): Json<ExternalImage>) -> Result<Json<Value>> {
	let url = externalImage.url;

	let kind = externalImage.kind.ok_or(RsError::Error("Missing image type".to_string()))?;

	let mut reader = mc.url_to_reader(&library_id, url, &user).await?;

	mc.update_serie_image(&library_id, &serie_id, &kind, reader.stream, &user).await?;
	
    Ok(Json(json!({"data": "ok"})))
}

#[debug_handler]
async fn handler_post_image(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageUploadOptions>, mut multipart: Multipart) -> Result<Json<Value>> {
	while let Some(field) = multipart.next_field().await.unwrap() {
        //let name = field.name().unwrap().to_string();
		//let filename = field.file_name().unwrap().to_string();
		//let mime: String = field.content_type().unwrap().to_string();
        //let data = field.bytes().await.unwrap();
		let mut reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		// Read all bytes from the field into a buffer
		let mut data = Vec::new();
		tokio::io::copy(&mut reader, &mut data).await?;
		let reader = Box::pin(Cursor::new(data));
        //println!("Length of `{}` {}  {} is {} bytes", name, filename, mime, data.len());
		mc.update_serie_image(&library_id, &serie_id, &query.kind, reader, &user).await?;
		

    }
	
    Ok(Json(json!({"data": "ok"})))
}