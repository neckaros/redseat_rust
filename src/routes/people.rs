
use crate::{model::{people::{PeopleQuery, PersonForAdd, PersonForUpdate}, users::ConnectedUser, ModelController}, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use futures::TryStreamExt;
use serde_json::{json, Value};
use tokio_util::io::{ReaderStream, StreamReader};
use crate::Error;

use super::ImageRequestOptions;


pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/", post(handler_post))
		.route("/:id", get(handler_get))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))
		.route("/:id/image", get(handler_image))
		.route("/:id/image", post(handler_post_image))
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
async fn handler_image(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Response> {
	let reader_response = mc.person_image(&library_id, &tag_id, query.kind, query.size, &user).await?;

	let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);
	
    Ok((headers, body).into_response())
}

async fn handler_post_image(Path((library_id, person_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, mut multipart: Multipart) -> Result<Json<Value>> {
	while let Some(field) = multipart.next_field().await.unwrap() {
		let reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		mc.update_person_image(&library_id, &person_id, &None, reader, &user).await?;
    }
	
    Ok(Json(json!({"data": "ok"})))
}

async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(tag): Json<PersonForAdd>) -> Result<Json<Value>> {
	let credential = mc.add_pesron(&library_id, tag, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}