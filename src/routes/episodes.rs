
use crate::{domain::{episode::{self, Episode}, media::{FileEpisode, Media, MediaForUpdate}, progress, view_progress::{ViewProgressForAdd, ViewProgressLigh}, watched::{WatchedForAdd, WatchedLight}, MediasIds}, model::{episodes::{EpisodeForUpdate, EpisodeQuery}, medias::MediaQuery, users::{ConnectedUser, HistoryQuery}, ModelController}, tools::image_tools::ImageType, Error, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use futures::TryStreamExt;
use rs_plugin_common_interfaces::{lookup::{RsLookupEpisode, RsLookupQuery}, request::RsRequest, MediaType};
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
	let serie = mc.get_serie(&library_id, serie_id,  &user).await?.ok_or(Error::NotFound)?;
	let query_episode = RsLookupEpisode {
    serie: serie.name,
    imdb: serie.imdb,
    slug: serie.slug,
    tmdb: serie.tmdb,
    trakt: serie.trakt,
    tvdb: serie.tmdb,
    otherids: serie.otherids,
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
	let serie = mc.get_serie(&library_id, serie_id,  &user).await?.ok_or(Error::NotFound)?;
	let query_episode = RsLookupEpisode {
    serie: serie.name,
    imdb: episode.imdb,
    slug: episode.slug,
    tmdb: episode.tmdb,
    trakt: episode.trakt,
    tvdb: episode.tmdb,
    otherids: episode.otherids,
    season: episode.season,
    number: Some(episode.number),
	};
	let query = RsLookupQuery::Episode(query_episode);
	let library = mc.exec_lookup(query, Some(library_id), &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_lookup_add(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Json(request): Json<RsRequest>) -> Result<Json<Value>> {
	let infos = MediaForUpdate {
		add_series: Some(vec![FileEpisode {
			id: serie_id,
			season: Some(season),
			episode: Some(number),
		}]),
		..Default::default()
	};
	let added = mc.medias_add_request(&library_id,  request, Some(infos), &user).await.expect("Unable to download");

	
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
	let progress = mc.get_view_progress(episode.into(), &user).await?.ok_or(Error::NotFound)?;
	Ok(Json(json!(progress)))
}

async fn handler_progress_set(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Json(progress): Json<ViewProgressLigh>) -> Result<()> {
	let episode = mc.get_episode(&library_id, serie_id.clone(), season, number, &user).await?;
	let serie = mc.get_serie(&library_id, serie_id, &user).await?.ok_or(Error::NotFound)?;
	let id = MediasIds::from(episode).into_best_external().ok_or(Error::NotFound)?;
	let serie_id = MediasIds::from(serie).into_best_external().ok_or(Error::NotFound)?;
	let progress = ViewProgressForAdd { kind: MediaType::Episode, id, progress: progress.progress, parent: Some(serie_id) };
	mc.add_view_progress(progress, &user).await?;

	Ok(())
}

async fn handler_watched_get(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let episode = mc.get_episode(&library_id, serie_id, season, number, &user).await?;
	let query = HistoryQuery {
		id: Some(episode.into()),
		..Default::default()
	};
	let progress = mc.get_watched(query, &user).await?.into_iter().next().ok_or(Error::NotFound)?;
	Ok(Json(json!(progress)))
}

async fn handler_watched_set(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Json(watched): Json<WatchedLight>) -> Result<()> {
	let episode = mc.get_episode(&library_id, serie_id.clone(), season, number, &user).await?;
	let id = MediasIds::from(episode).into_best_external_or_local().ok_or(Error::NotFound)?;
	let watched = WatchedForAdd { kind: MediaType::Episode, id, date: watched.date };
	mc.add_watched(watched, &user).await?;

	Ok(())
}



async fn handler_image(Path((library_id, serie_id, season, number)): Path<(String, String, u32, u32)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Response> {
	let reader_response = mc.episode_image(&library_id, &serie_id, &season, &number, query.size.clone(), &user).await;

	if let Ok(reader_response) = reader_response {
		let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
		let stream = ReaderStream::new(reader_response.stream);
		let body = Body::from_stream(stream);
		Ok((headers, body).into_response())
	} else if let Ok(reader_response) = mc.serie_image(&library_id, &serie_id, Some(ImageType::Card), query.size.clone(), &user).await {
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