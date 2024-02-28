
use std::{path::PathBuf, pin::Pin};

use crate::{domain::{backup, library, tag}, model::{backups::{BackupForAdd, BackupForUpdate}, credentials::{CredentialForAdd, CredentialForUpdate}, libraries::ServerLibraryForUpdate, people::{PeopleQuery, PersonForAdd, PersonForUpdate}, tags::{TagForAdd, TagForUpdate, TagQuery}, users::ConnectedUser, ModelController}, plugins::sources::{path_provider::PathProvider, virtual_provider::VirtualProvider, Source}, Result};
use axum::{body::Body, debug_handler, extract::{Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use http_body_util::StreamBody;
use hyper::{header, HeaderMap, Request, StatusCode};
use mime::APPLICATION_OCTET_STREAM;
use serde_json::{json, Value};
use tokio_util::io::ReaderStream;
use tower_http::services::ServeFile;
use crate::Error;


pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/", post(handler_post))
		.route("/:id", get(handler_get))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))
		.route("/:id/image", get(handler_image))
		.with_state(mc)
        
}

async fn handler_list(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<PeopleQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_people(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_get(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_person(&library_id, tag_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_patch(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<PersonForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_person(&library_id, tag_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_person(&library_id, &tag_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

#[debug_handler]
async fn handler_image(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, headers: HeaderMap) -> Result<Response> {
	let m = mc.source_for_library(&library_id).await?;
	let reader_response = m.get_file_read_stream(format!(".redseat\\.portraits\\{}.webp", tag_id)).await.map_err(|e| Error::GenericRedseatError)?;

	let headers = reader_response.hearders().map_err(|e| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);
	
    Ok((headers, body).into_response())
}

async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(tag): Json<PersonForAdd>) -> Result<Json<Value>> {
	let credential = mc.add_pesron(&library_id, tag, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}