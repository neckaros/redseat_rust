
use crate::{domain::plugin::{PluginForAdd, PluginForInstall, PluginForUpdate}, model::{plugins::PluginQuery, users::ConnectedUser}, ModelController, Result};
use axum::{extract::{Path, Query, State}, routing::{delete, get, patch, post}, Json, Router};
use plugin_request_interfaces::RsRequest;
use rs_plugin_common_interfaces::PluginType;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use rs_plugin_url_interfaces::RsLink;



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/install", post(handler_install))

		.route("/parse", get(handler_parse))
		.route("/expand", post(handler_expand))

		.route("/urlrequest", get(handler_urlrequest))
		.route("/", post(handler_post))
		.route("/:id", get(handler_get))
		.route("/:id/reload", get(handler_reload))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))
		.with_state(mc)
        
}

async fn handler_list(State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<PluginQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_plugins(query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_install(State(mc): State<ModelController>, user: ConnectedUser, Json(plugin): Json<PluginForInstall>) -> Result<Json<Value>> {
	let libraries = mc.install_plugin(plugin, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ExpandQuery {
	pub url: String,
}


async fn handler_parse(State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ExpandQuery>) -> Result<Json<Value>> {
	user.check_role(&crate::model::users::UserRole::Read)?;
	let wasm = mc.exec_parse(None, query.url, &user).await?;
	let body = Json(json!(wasm));
	Ok(body)
}
async fn handler_expand(State(mc): State<ModelController>, user: ConnectedUser, Json(link): Json<RsLink>) -> Result<Json<Value>> {
	user.check_role(&crate::model::users::UserRole::Read)?;

	let wasm = mc.exec_expand(None, link, &user).await?;

	let body = Json(json!(wasm));
	Ok(body)
}

async fn handler_urlrequest(State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ExpandQuery>) -> Result<Json<Value>> {
	let request = RsRequest {
		url: query.url,
		..Default::default()
	};
	let wasm = mc.exec_request(request, None, false, None, &user).await?;
	let body = match wasm {
		crate::plugins::sources::SourceRead::Stream(_) => Json(json!({"stream": true})),
		crate::plugins::sources::SourceRead::Request(r) => Json(json!(r)),
	};
	Ok(body)
}

async fn handler_get(Path(plugin_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_plugin(plugin_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}


async fn handler_reload(Path(plugin_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.reload_plugin(plugin_id, &user).await?;
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