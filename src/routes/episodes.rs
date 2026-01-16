
use std::io::Cursor;

use crate::{domain::{episode::{self, Episode}, media::{FileEpisode, Media, MediaForUpdate}, progress, view_progress::{ViewProgressForAdd, ViewProgressLigh}, watched::{WatchedForAdd, WatchedLight}}, error::RsError, model::{episodes::{EpisodeForUpdate, EpisodeQuery}, medias::MediaQuery, users::{ConnectedUser, HistoryQuery}, ModelController}, plugins::sources::error::SourcesError, Error, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use futures::TryStreamExt;
use rs_plugin_common_interfaces::{domain::rs_ids::RsIds, lookup::{RsLookupEpisode, RsLookupQuery}, request::{RsGroupDownload, RsRequest}, ImageType, MediaType};
use serde_json::{json, ser, Value};
use tokio::io::AsyncRead;
use tokio_util::io::{ReaderStream, StreamReader};

use super::{ImageRequestOptions, ImageUploadOptions};


pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/episodes", get(handler_list))
		.route("/episodes", post(handler_post))
		.route("/episodes/refresh", get(handler_refresh))
		
		.route("/seasons/:season/search", get(handler_lookup_season))
		.route("/seasons/:season/episodes", get(handler_list_season_episodes))
		.route("/seasons/:season/episodes/:number", get(handler_get))
		.route("/seasons/:season/episodes/:number", post(handler_post_episode))
		.route("/seasons/:season/episodes/:number", patch(handler_patch))
		.route("/seasons/:season/episodes/:number", delete(handler_delete))
		.route("/seasons/:season/episodes/:number/image", get(handler_image))
		.route("/seasons/:season/episodes/:number/search", get(handler_lookup))
		.route("/seasons/:season/episodes/:number/search", post(handler_lookup_add))
		.route("/seasons/:season/episodes/:number/medias", get(handler_medias))
		.route("/seasons/:season/episodes/:number/progress", get(handler_progress_get))
		.route("/seasons/:season/episodes/:number/progress", post(handler_progress_set))
		.route("/seasons/:season/episodes/:number/watched", get(handler_watched_get))
		.route("/seasons/:season/episodes/:number/watched", post(handler_watched_set))
		.route("/:id/image", post(handler_post_image))
		.with_state(mc)
        
}

async fn handler_list(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(mut query): Query<EpisodeQuery>) -> Result<Json<Value>> {
	query.serie_ref = Some(serie_id);
	let libraries = mc.get_episodes(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}
async fn handler_refresh(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(_query): Query<EpisodeQuery>) -> Result<Json<Value>> {
	let libraries = mc.refresh_episodes(&library_id, &serie_id, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_lookup_season(Path((library_id, serie_id, season)): Path<(String, String, u32)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let serie = mc.get_serie(&library_id.clone(), serie_id.clone(),  &user).await?.ok_or(SourcesError::UnableToFindSerie(library_id.to_string(), serie_id.to_string(), "handler_lookup_season".to_string()))?;
	let name = serie.name.clone();
	let ids: RsIds = serie.into();
	let query_episode = RsLookupEpisode {
    serie: name,
    ids: Some(ids),
	season,
	number: None
	};
	let query = RsLookupQuery::Episode(query_episode);
	let library = mc.exec_lookup(query, Some(library_id), &user).await?;
	let body = Json(json!(library));
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

async fn handler_post_episode(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Json(mut insert): Json<Episode>) -> Result<Json<Value>> {
	insert.serie = serie_id;
	insert.season = season;
	insert.number = number;
	let new_credential = mc.add_episode(&library_id, insert,  &user).await?;
	Ok(Json(json!(new_credential)))
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

async fn handler_lookup(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let episode = mc.get_episode(&library_id, serie_id.clone(), season, number, &user).await?;
	let serie = mc.get_serie(&library_id, serie_id.clone(),  &user).await?.ok_or(SourcesError::UnableToFindSerie(library_id.to_string(), serie_id.to_string(), "handler_lookup".to_string()))?;
	let name = serie.name.clone();
	let ids: RsIds = serie.into();
	let query_episode = RsLookupEpisode {
    serie: name,
    ids: Some(ids),
    season: episode.season,
    number: Some(episode.number),
	};
	let query = RsLookupQuery::Episode(query_episode);
	let library = mc.exec_lookup(query, Some(library_id), &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_lookup_add(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Json(mut request): Json<RsRequest>) -> Result<Json<Value>> {
	// Set series info directly on the request
	request.albums = Some(vec![serie_id]);
	request.season = Some(season);
	request.episode = Some(number);

	let group = RsGroupDownload {
		requests: vec![request],
		group: false,
		..Default::default()
	};
	let added = mc.download_library_url(&library_id, group, &user).await?;
	let added = added.into_iter().next().ok_or(Error::Error("No media added".to_string()))?;

	Ok(Json(json!(added)))
}



async fn handler_medias(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.get_medias(&library_id, MediaQuery { series: vec![format!("{}|{:04}|{:04}", serie_id, season, number)], ..Default::default() }, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_post(Path((library_id, serie_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(mut insert): Json<Episode>) -> Result<Json<Value>> {
	insert.serie = serie_id;
	let credential = mc.add_episode(&library_id, insert, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}


async fn handler_progress_get(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let episode = mc.get_episode(&library_id, serie_id, season, number, &user).await?;
	let progress = mc.get_view_progress(episode.into(), &user, Some(library_id.to_string())).await?.ok_or(Error::NotFound(format!("Unable to get best view progress for handler_progress_get")))?;
	Ok(Json(json!(progress)))
}

async fn handler_progress_set(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Json(progress): Json<ViewProgressLigh>) -> Result<()> {
	let episode = mc.get_episode(&library_id, serie_id.clone(), season, number, &user).await?;
	let serie = mc.get_serie(&library_id, serie_id.clone(), &user).await?.ok_or(SourcesError::UnableToFindSerie(library_id.to_string(), serie_id.to_string(), "handler_lookup_season".to_string()))?;
	let id = RsIds::from(episode).into_best_external().ok_or(Error::NotFound(format!("Unable to get best external for handler_progress_set")))?;
	let serie_id = RsIds::from(serie).into_best_external().ok_or(Error::NotFound(format!("Unable to get best external for handler_progress_set serie")))?;
	let progress = ViewProgressForAdd { kind: MediaType::Episode, id, progress: progress.progress, parent: Some(serie_id) };
	mc.add_view_progress(progress, &user, Some(library_id)).await?;

	Ok(())
}

async fn handler_watched_get(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let episode = mc.get_episode(&library_id, serie_id, season, number, &user).await?;
	let query = HistoryQuery {
		id: Some(episode.into()),
		..Default::default()
	};
	let progress = mc.get_watched(query, &user, Some(library_id)).await?.into_iter().next().ok_or(Error::NotFound(format!("Unable to get best watched")))?;
	Ok(Json(json!(progress)))
}

async fn handler_watched_set(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Json(watched): Json<WatchedLight>) -> Result<()> {
	let episode = mc.get_episode(&library_id, serie_id.clone(), season, number, &user).await?;
	let id = RsIds::from(episode).into_best_external_or_local().ok_or(Error::NotFound(format!("Unable to get best external for handler_watched_set")))?;
	let watched = WatchedForAdd { kind: MediaType::Episode, id, date: watched.date };
	mc.add_watched(watched, &user, Some(library_id)).await?;

	Ok(())
}



async fn handler_image(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Response> {
	let reader_response = mc.episode_image(&library_id, &serie_id, &season, &number, query.size.clone(), &user).await;

	if let Ok(reader_response) = reader_response {
		let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
		let stream = ReaderStream::new(reader_response.stream);
		let body = Body::from_stream(stream);
		Ok((headers, body).into_response())
	} else if query.defaulting { 
		if let Ok(reader_response) = mc.serie_image(&library_id, &serie_id, Some(ImageType::Card), query.size.clone(), &user).await {
			let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
			let stream = ReaderStream::new(reader_response.stream);
			let body = Body::from_stream(stream);
			Ok((headers, body).into_response())
		} else {
			let reader_response = mc.serie_image(&library_id, &serie_id, Some(ImageType::Background), query.size, &user).await?;
			let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
			let stream = ReaderStream::new(reader_response.stream);
			let body = Body::from_stream(stream);
			Ok((headers, body).into_response())
		}
	} else if let Err(err) = reader_response {
		Err(Error::NotFound(format!("Unable to find episode image with defaulting: {} {} {:?}", library_id, serie_id, err)))
	} else {
		Err(Error::NotFound(format!("Unable to find episode image with defaulting: {} {}", library_id, serie_id)))
	}
	
	
}

#[debug_handler]
async fn handler_post_image(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageUploadOptions>, mut multipart: Multipart) -> Result<Json<Value>> {
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
		mc.update_serie_image(&library_id, &tag_id, &query.kind, reader, &user).await?;
    }
	
    Ok(Json(json!({"data": "ok"})))
}