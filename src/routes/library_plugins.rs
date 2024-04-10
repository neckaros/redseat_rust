
use crate::{domain::plugin::{PluginForAdd, PluginForInstall, PluginForUpdate}, model::{plugins::PluginQuery, users::ConnectedUser}, ModelController, Result};
use axum::{extract::{Path, Query, State}, routing::{delete, get, patch, post}, Json, Router};
use plugin_request_interfaces::RsRequest;
use rs_plugin_common_interfaces::PluginType;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use rs_plugin_url_interfaces::RsLink;



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/url/parse", get(handler_parse))
		.route("/url/expand", post(handler_expand))

		.route("/requests/process", post(handler_request_process))
		.route("/requests/url", get(handler_request_url))

		.with_state(mc)
        
}



#[derive(Debug, Serialize, Deserialize, Clone)]
struct ExpandQuery {
	pub url: String,
}


async fn handler_parse(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ExpandQuery>) -> Result<Json<Value>> {
	user.check_role(&crate::model::users::UserRole::Read)?;
	let wasm = mc.exec_parse(Some(library_id), query.url, &user).await?;
	let body = Json(json!(wasm));
	Ok(body)
}
async fn handler_expand(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(link): Json<RsLink>) -> Result<Json<Value>> {
	user.check_role(&crate::model::users::UserRole::Read)?;

	let wasm = mc.exec_expand(Some(library_id), link, &user).await?;

	let body = Json(json!(wasm));
	Ok(body)
}


async fn handler_request_url(State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ExpandQuery>) -> Result<Json<Value>> {
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

async fn handler_request_process(State(mc): State<ModelController>, user: ConnectedUser, Json(request): Json<RsRequest>) -> Result<Json<Value>> {
	let wasm = mc.exec_request(request, None, false, None, &user).await?;
	let body = match wasm {
		crate::plugins::sources::SourceRead::Stream(_) => Json(json!({"stream": true})),
		crate::plugins::sources::SourceRead::Request(r) => Json(json!(r)),
	};
	Ok(body)
}