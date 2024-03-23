
use crate::{domain::episode::Episode, model::{episodes::{EpisodeForUpdate, EpisodeQuery}, users::ConnectedUser, ModelController}, Error, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use futures::TryStreamExt;
use serde_json::{json, ser, Value};
use tokio::io::AsyncRead;
use tokio_util::io::{ReaderStream, StreamReader};

use super::{ImageRequestOptions, ImageUploadOptions};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/episodes", get(handler_list))
		.route("/episodes", post(handler_post))
		.route("/episodes/refresh", get(handler_refresh))
		.route("/seasons/:season/episodes", get(handler_list_season_episodes))
		.route("/seasons/:season/episodes/:number", get(handler_get))
		.route("/seasons/:season/episodes/:number", patch(handler_patch))
		.route("/seasons/:season/episodes/:number", delete(handler_delete))
		.route("/seasons/:season/episodes/:number/image", get(handler_image))
		.route("/:id/image", post(handler_post_image))
		.with_state(mc)
        
}

async fn handler_list(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(mut query): Query<EpisodeQuery>) -> Result<Json<Value>> {
	query.serie_ref = Some(serie_id);
	let libraries = mc.get_episodes(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}
async fn handler_refresh(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(mut query): Query<EpisodeQuery>) -> Result<Json<Value>> {
	let libraries = mc.refresh_episodes(&library_id, &serie_id, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_list_season_episodes(Path((library_id, serie_id, season)): Path<(String, String, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Query(mut query): Query<EpisodeQuery>) -> Result<Json<Value>> {
	query.serie_ref = Some(serie_id);
	query.season = Some(season);
	let libraries = mc.get_episodes(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_get(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_episode(&library_id, serie_id, season, number, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_patch(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<EpisodeForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_episode(&library_id, serie_id, season, number, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_episode(&library_id, &serie_id, season, number, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(Path((library_id, _)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(new_serie): Json<Episode>) -> Result<Json<Value>> {
	let credential = mc.add_episode(&library_id, new_serie, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}


async fn handler_image(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Response> {
	let reader_response = mc.episode_image(&library_id, &serie_id, &season, &number, query.size, &user).await?;

	let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);
	
    Ok((headers, body).into_response())
}

#[debug_handler]
async fn handler_post_image(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageUploadOptions>, mut multipart: Multipart) -> Result<Json<Value>> {
	while let Some(field) = multipart.next_field().await.unwrap() {
        //let name = field.name().unwrap().to_string();
		//let filename = field.file_name().unwrap().to_string();
		//let mime: String = field.content_type().unwrap().to_string();
        //let data = field.bytes().await.unwrap();

		let reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		
        //println!("Length of `{}` {}  {} is {} bytes", name, filename, mime, data.len());
			mc.update_serie_image(&library_id, &tag_id, &query.kind, reader, &user).await?;
    }
	
    Ok(Json(json!({"data": "ok"})))
}