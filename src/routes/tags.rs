
use crate::{model::{tags::{TagForAdd, TagForUpdate, TagQuery}, users::ConnectedUser, ModelController}, Result};
use axum::{extract::{Path, Query, State}, routing::{delete, get, patch, post}, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/", post(handler_post))
		.route("/:id", get(handler_get))
		.route("/:id", patch(handler_patch))
		.route("/:id/merge", patch(handler_merge))
		.route("/:id", delete(handler_delete))
		.with_state(mc)
        
}

async fn handler_list(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<TagQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_tags(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_get(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_tag(&library_id, tag_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_patch(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<TagForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_tag(&library_id, tag_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct MergeOption {
	pub into: String
}


async fn handler_merge(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<MergeOption>) -> Result<Json<Value>> {
	let new_credential = mc.merge_tag(&library_id, tag_id, update.into, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_tag(&library_id, &tag_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(tag): Json<TagForAdd>) -> Result<Json<Value>> {
	println!("insert");
	let credential = mc.add_tag(&library_id, tag, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}