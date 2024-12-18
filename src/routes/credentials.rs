
use crate::{model::{credentials::{CredentialForAdd, CredentialForUpdate}, users::ConnectedUser, ModelController}, Result};
use axum::{extract::{Path, State}, routing::{delete, get, patch, post}, Json, Router};
use serde_json::{json, Value};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
	.route("/available", get(handler_available))
	.route("/", get(handler_list))
	.route("/", post(handler_post))
	.route("/:id", get(handler_get))
	.route("/:id", patch(handler_patch))
	.route("/:id", delete(handler_delete))
	.with_state(mc)
        
}

async fn handler_available(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.get_credentials_available(&user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}


async fn handler_list(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.get_credentials(&user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_get(Path(credential_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_credential(credential_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_patch(Path(credential_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<CredentialForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_credential(&credential_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path(credential_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_credential(&credential_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(State(mc): State<ModelController>, user: ConnectedUser, Json(credential): Json<CredentialForAdd>) -> Result<Json<Value>> {
	let credential = mc.add_credential(credential, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}