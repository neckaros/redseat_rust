
use crate::{domain::{media::MediaForUpdate, movie::{Movie, MovieForUpdate}, view_progress::{ViewProgressForAdd, ViewProgressLigh}, watched::{WatchedForAdd, WatchedLight}, MediasIds}, error::RsError, model::{episodes::EpisodeQuery, medias::MediaQuery, movies::{MovieQuery, RsMovieSort}, store::sql::SqlOrder, users::{ConnectedUser, HistoryQuery}, ModelController}, tools::{clock::now, image_tools::ImageType}, Error, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post, put}, Json, Router};
use futures::TryStreamExt;
use rs_plugin_common_interfaces::{lookup::{RsLookupMovie, RsLookupQuery}, MediaType, RsRequest};
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
		.route("/", post(handler_post))
		.route("/search", get(handler_seach_movies))
		.route("/:id", get(handler_get))
		
		.route("/:id/medias", get(handler_medias))
		.route("/:id/search", get(handler_lookup))
		.route("/:id/search", post(handler_lookup_add))
		.route("/:id", patch(handler_patch))
		.route("/:id/import", put(handler_import))
		.route("/:id/refresh", get(handler_refresh))
		.route("/:id", delete(handler_delete))
		.route("/:id/image", get(handler_image))
		.route("/:id/image", post(handler_post_image))
		.route("/:id/progress", get(handler_progress_get))
		.route("/:id/progress", post(handler_progress_set))
		.route("/:id/watched", get(handler_watched_get))
		.route("/:id/watched", post(handler_watched_set))
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

async fn handler_upcoming(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.get_movies(&library_id, MovieQuery { in_digital: Some(false), sort: RsMovieSort::Digitalairdate, order: Some(SqlOrder::ASC), ..Default::default() }, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}
async fn handler_ondeck(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.get_movies(&library_id, MovieQuery { in_digital: Some(true), watched: Some(false), sort: RsMovieSort::Digitalairdate, order: Some(SqlOrder::DESC), ..Default::default() }, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_get(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let movie = mc.get_movie(&library_id, movie_id, &user).await?;
	let body = Json(json!(movie));
	Ok(body)
}

async fn handler_seach_movies(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<RsLookupMovie>) -> Result<Json<Value>> {
	let libraries = mc.search_movie(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}


async fn handler_medias(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.get_medias(&library_id, MediaQuery { movie: Some(movie_id), ..Default::default() }, &user).await?;
	let body = Json(json!(libraries));
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

async fn handler_lookup_add(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(request): Json<RsRequest>) -> Result<Json<Value>> {
	let infos = MediaForUpdate {
		movie: Some(movie_id),
		..Default::default()
	};
	let added = mc.medias_add_request(&library_id,  request, Some(infos), &user).await.expect("Unable to download");

	
	Ok(Json(json!(added)))
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




async fn handler_progress_get(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let movie = mc.get_movie(&library_id, movie_id, &user).await?;
	let progress = mc.get_view_progress(movie.into(), &user, Some(library_id.to_string())).await?.ok_or(Error::NotFound)?;
	Ok(Json(json!(progress)))
}

async fn handler_progress_set(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(progress): Json<ViewProgressLigh>) -> Result<()> {
	let movie = mc.get_movie(&library_id, movie_id, &user).await?;
	let id = MediasIds::from(movie).into_best_external().ok_or(Error::NotFound)?;
	let progress = ViewProgressForAdd { kind: MediaType::Movie, id, progress: progress.progress, parent: None };
	mc.add_view_progress(progress, &user, Some(library_id)).await?;

	Ok(())
}

async fn handler_watched_get(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let movie = mc.get_movie(&library_id, movie_id, &user).await?;
	let query = HistoryQuery {
		id: Some(movie.into()),
		..Default::default()
	};
	let progress = mc.get_watched(query, &user, Some(library_id)).await?.into_iter().next().ok_or(Error::NotFound)?;
	Ok(Json(json!(progress)))
}

async fn handler_watched_set(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(watched): Json<WatchedLight>) -> Result<()> {
	let movie = mc.get_movie(&library_id, movie_id, &user).await?;
	let id = MediasIds::from(movie).into_best_external().ok_or(Error::NotFound)?;
	let watched = WatchedForAdd { kind: MediaType::Movie, id, date: watched.date };
	mc.add_watched(watched, &user, Some(library_id)).await?;

	Ok(())
}



async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(tag): Json<Movie>) -> Result<Json<Value>> {
	let credential = mc.add_movie(&library_id, tag, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}


async fn handler_image(Path((library_id, movie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Response> {
	let reader_response = mc.movie_image(&library_id, &movie_id, query.kind.clone(), query.size.clone(), &user).await;

	if let Ok(reader_response) =reader_response {
		let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
		let stream = ReaderStream::new(reader_response.stream);
		let body = Body::from_stream(stream);
		
		Ok((headers, body).into_response())
	} else if query.defaulting {
		if query.kind.as_ref().unwrap_or(&ImageType::Poster) == &ImageType::Card {
			let reader_response = mc.movie_image(&library_id, &movie_id, Some(ImageType::Background), query.size, &user).await?;
			let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
			let stream = ReaderStream::new(reader_response.stream);
			let body = Body::from_stream(stream);
			
			Ok((headers, body).into_response())
		} else {
			Err(Error::NotFound)
		}
	} else {
		Err(RsError::NotFound)
	}

	
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