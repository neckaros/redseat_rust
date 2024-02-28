
use crate::{domain::backup, model::{backups::{BackupForAdd, BackupForUpdate}, credentials::{CredentialForAdd, CredentialForUpdate}, libraries::ServerLibraryForUpdate, series::{SerieForAdd, SerieForUpdate, SerieQuery}, tags::{TagForAdd, TagForUpdate, TagQuery}, users::ConnectedUser, ModelController}, Error, Result};
use axum::{body::Body, extract::{Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use hyper::HeaderMap;
use serde_json::{json, Value};
use tokio_util::io::ReaderStream;



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

async fn handler_list(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<SerieQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_series(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_get(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_serie(&library_id, tag_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_patch(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<SerieForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_serie(&library_id, tag_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_serie(&library_id, &tag_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(tag): Json<SerieForAdd>) -> Result<Json<Value>> {
	let credential = mc.add_serie(&library_id, tag, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}


async fn handler_image(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, headers: HeaderMap) -> Result<Response> {
	let reader_response = mc.serie_image(&library_id, &tag_id, &user).await?;

	let headers = reader_response.hearders().map_err(|e| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);
	
    Ok((headers, body).into_response())
}