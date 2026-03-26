
use crate::{model::{users::{ConnectedUser, UploadKeyForCreate}, ModelController}, Result};
use axum::{extract::{Path, State}, routing::{delete, get, post}, Json, Router};
use serde_json::{json, Value};

pub fn routes(mc: ModelController) -> Router {
	Router::new()
	.route("/", get(handler_list))
	.route("/", post(handler_post))
	.route("/:id", delete(handler_delete))
	.with_state(mc)
}

async fn handler_list(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let keys = mc.get_upload_keys(&user).await?;
	Ok(Json(json!(keys)))
}

async fn handler_post(State(mc): State<ModelController>, user: ConnectedUser, Json(params): Json<UploadKeyForCreate>) -> Result<Json<Value>> {
	let key = mc.add_upload_key(params, &user).await?;
	Ok(Json(json!(key)))
}

async fn handler_delete(Path(key_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	mc.remove_upload_key(&key_id, &user).await?;
	Ok(Json(json!({"deleted": key_id})))
}
