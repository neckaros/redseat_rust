
use crate::{domain::movie::{Movie, MovieForUpdate}, model::{episodes::EpisodeQuery, movies::MovieQuery, users::ConnectedUser, ModelController}, Error, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post, put}, Json, Router};
use futures::TryStreamExt;
use rs_plugin_lookup_interfaces::{RsLookupMovie, RsLookupQuery};
use serde_json::{json, Value};
use tokio::io::AsyncRead;
use tokio_util::io::{ReaderStream, StreamReader};

use super::{ImageRequestOptions, ImageUploadOptions};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/trending", get(handler_trending))
		.route("/upcoming", get(handler_upcoming))
		.route("/", post(handler_post))
		.route("/:id", get(handler_get))
		.route("/:id/search", get(handler_lookup))
		.route("/:id", patch(handler_patch))
		.route("/:id/import", put(handler_import))
		.route("/:id/refresh", get(handler_refresh))
		.route("/:id", delete(handler_delete))
		.route("/:id/image", get(handler_image))
		.route("/:id/image", post(handler_post_image))
		.with_state(mc.clone())
        
}

async fn handler_list(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<MovieQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_movies(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_trending(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.trending_movies(&user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_upcoming(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<EpisodeQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_episodes_upcoming(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_get(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let movie = mc.get_movie(&library_id, movie_id, &user).await?;
	let body = Json(json!(movie));
	Ok(body)
}

async fn handler_lookup(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let movie = mc.get_movie(&library_id, movie_id, &user).await?;
	let query = RsLookupQuery::Movie(RsLookupMovie {
		name: movie.name,
		imdb: movie.imdb,
		slug: movie.slug,
		tmdb: movie.tmdb,
		trakt: movie.trakt,
		otherids: movie.otherids,
	});
	let library = mc.exec_lookup(query, Some(library_id), &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_refresh(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.refresh_movie(&library_id, &movie_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_patch(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<MovieForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_movie(&library_id, movie_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_import(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let new_credential = mc.import_movie(&library_id, &movie_id, &user).await?;
	Ok(Json(json!(new_credential)))
}


async fn handler_delete(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_movie(&library_id, &movie_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(tag): Json<Movie>) -> Result<Json<Value>> {
	let credential = mc.add_movie(&library_id, tag, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}


async fn handler_image(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Response> {
	let reader_response = mc.movie_image(&library_id, &movie_id, query.kind, query.size, &user).await?;

	let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);
	
    Ok((headers, body).into_response())
}

#[debug_handler]
async fn handler_post_image(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageUploadOptions>, mut multipart: Multipart) -> Result<Json<Value>> {
	while let Some(field) = multipart.next_field().await.unwrap() {
        //let name = field.name().unwrap().to_string();
		//let filename = field.file_name().unwrap().to_string();
		//let mime: String = field.content_type().unwrap().to_string();
        //let data = field.bytes().await.unwrap();

		let reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		
        //println!("Length of `{}` {}  {} is {} bytes", name, filename, mime, data.len());
			mc.update_movie_image(&library_id, &movie_id, &query.kind, reader, &user).await?;
    }
	
    Ok(Json(json!({"data": "ok"})))
}