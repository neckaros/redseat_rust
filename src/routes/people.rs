
use crate::{model::{people::{PeopleQuery, PersonForAdd, PersonForUpdate}, users::ConnectedUser, ModelController}, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use serde_json::{json, Value};
use tokio_util::io::ReaderStream;
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

async fn handler_post_image(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>, mut multipart: Multipart) -> Result<Json<Value>> {
	while let Some(mut field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_string();
		let filename = field.file_name().unwrap().to_string();
		let mime: String = field.content_type().unwrap().to_string();
        let data = field.bytes().await.unwrap();

        println!("Length of `{}` {}  {} is {} bytes", name, filename, mime, data.len());
    }
	
    Ok(Json(json!({"data": "ok"})))
}

async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(tag): Json<PersonForAdd>) -> Result<Json<Value>> {
	let credential = mc.add_pesron(&library_id, tag, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}