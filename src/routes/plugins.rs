
use crate::{domain::plugin::{PluginForAdd, PluginForUpdate}, model::{plugins::PluginQuery, users::ConnectedUser}, ModelController, Result};
use axum::{extract::{Path, Query, State}, routing::{delete, get, patch, post}, Json, Router};
use serde_json::{json, Value};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/", post(handler_post))
		.route("/:id", get(handler_get))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))
		.with_state(mc)
        
}

async fn handler_list(State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<PluginQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_plugins(query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_get(Path(plugin_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_plugin(plugin_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_patch(Path(plugin_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<PluginForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_plugin(&plugin_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path(plugin_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_plugin(&plugin_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(State(mc): State<ModelController>, user: ConnectedUser, Json(plugin): Json<PluginForAdd>) -> Result<Json<Value>> {
	let credential = mc.add_plugin(plugin, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}