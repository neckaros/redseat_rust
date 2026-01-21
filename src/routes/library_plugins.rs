
use crate::{domain::{library::LibraryRole, plugin::{PluginForAdd, PluginForInstall, PluginForUpdate}}, model::{plugins::PluginQuery, users::ConnectedUser}, ModelController, Result};
use axum::{extract::{Path, Query, State}, response::Response, routing::{delete, get, patch, post}, Json, Router};
use rs_plugin_common_interfaces::{request::RsRequest, url::RsLink, video::RsVideoCapabilities, PluginType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::mw_range::RangeDefinition;



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/url/parse", get(handler_parse))
		.route("/url/expand", post(handler_expand))

		.route("/requests/process", post(handler_request_process))
		.route("/requests/process/stream", post(handler_request_process_stream))

		.route("/requests/permanent", post(handler_request_permanent))
		.route("/requests/url", get(handler_request_url))
		.route("/requests/url/stream", get(handler_request_url_stream))
		.route("/requests/url/sharetoken", get(handler_request_url_sharetoken))

		// Request processing routes
		.route("/requests/check-instant", post(handler_check_instant))
		.route("/requests/add", post(handler_request_add))
		.route("/requests/processing", get(handler_list_processing))
		.route("/requests/processing/:id", get(handler_get_processing))
		.route("/requests/processing/:id/progress", get(handler_get_processing_progress))
		.route("/requests/processing/:id/pause", post(handler_pause_processing))
		.route("/requests/processing/:id", delete(handler_remove_processing))

		.route("/videoconvert", get(handler_list_video_convert))
		.route("/videoconvert/:plugin_id/capabilities", get(handler_video_convert_caps))

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





async fn handler_request_url(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ExpandQuery>) -> Result<Json<Value>> {
	let request = RsRequest {
		url: query.url,
		..Default::default()
	};
	let wasm = mc.exec_request(request, Some(library_id), false, None, &user, None).await?;
	let body = match wasm {
		crate::plugins::sources::SourceRead::Stream(_) => return Err(crate::Error::Error("Request processing returned a stream instead of request info".to_string())),
		crate::plugins::sources::SourceRead::Request(r) => Json(json!(r)),
	};
	Ok(body)
}

async fn handler_request_url_stream(Path(library_id): Path<String>, range: Option<RangeDefinition>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ExpandQuery>) -> Result<Response> {
	let request = RsRequest { url: query.url, ..Default::default()};
	let wasm = mc.exec_request(request, Some(library_id.clone()), false, None, &user, None).await?;
	wasm.into_response(&library_id, range, None, Some((mc.clone(), &user))).await
}

async fn handler_request_url_sharetoken(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ExpandQuery>) -> Result<String> {
	let request = RsRequest { url: query.url, ..Default::default()};
	let sharetoken = mc.get_request_share_token(&library_id, &request, 6 * 60 * 60,  &user).await?;
	Ok(sharetoken)
}

async fn handler_request_process(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(request): Json<RsRequest>) -> Result<Json<Value>> {
	let wasm = mc.exec_request(request, Some(library_id), false, None, &user, None).await?;
	let body = match wasm {
		crate::plugins::sources::SourceRead::Stream(_) => return Err(crate::Error::Error("Request processing returned a stream instead of request info".to_string())),
		crate::plugins::sources::SourceRead::Request(r) => Json(json!(r)),
	};
	Ok(body)
}

async fn handler_request_process_stream(Path(library_id): Path<String>, range: Option<RangeDefinition>, State(mc): State<ModelController>, user: ConnectedUser, Json(request): Json<RsRequest>) -> Result<Response> {
	let wasm = mc.exec_request(request, Some(library_id.clone()), false, None, &user, None).await?;
	wasm.into_response(&library_id, range, None, Some((mc.clone(), &user))).await
}


async fn handler_request_permanent(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(request): Json<RsRequest>) -> Result<Json<Value>> {
	let request = mc.exec_permanent(request, Some(library_id), None, &user, None).await?;
	let body = Json(json!(request));
	Ok(body)
}



#[derive(Debug, Serialize, Deserialize, Clone)]
struct VideoConvertPlugin {
	pub id: String,
	pub name: String,
	pub credential: Option<String>,
	pub capabilities: Option<RsVideoCapabilities>,
}
async fn handler_list_video_convert(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	user.check_role(&crate::model::users::UserRole::Read)?;
	let wasm = mc.get_plugins_with_credential(PluginQuery { kind: Some(PluginType::VideoConvert), library: Some(library_id), ..Default::default() }).await?;
	let plugins = wasm.into_iter().map(|p| VideoConvertPlugin {
		id: p.plugin.id,
		name: p.plugin.name,
		credential: p.credential.map(|c| c.name),
		capabilities: None,
	}).collect::<Vec<VideoConvertPlugin>>();
	let body = Json(json!(plugins));
	Ok(body)
}
async fn handler_video_convert_caps(Path((library_id, plugin_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	user.check_role(&crate::model::users::UserRole::Read)?;
	let wasm = mc.get_plugin_with_credential(&plugin_id).await?;
	let caps = mc.plugin_manager.get_convert_capabilities(wasm).await?;
	let body = Json(json!(caps));
	Ok(body)
}

async fn handler_video_convert_status(Path((library_id, plugin_id, encode_id)): Path<(String, String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	user.check_role(&crate::model::users::UserRole::Read)?;
	let wasm = mc.get_plugin_with_credential(&plugin_id).await?;
	let status = mc.plugin_manager.convert_status(wasm, &encode_id).await?;
	let body = Json(json!(status));
	Ok(body)
}

// ============== Request Processing Handlers ==============

async fn handler_check_instant(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(request): Json<RsRequest>) -> Result<Json<Value>> {
	let result = mc.exec_check_instant(request, &library_id, &user, None).await?;
	Ok(Json(json!({ "instant": result })))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RequestAddBody {
	request: RsRequest,
	media_ref: Option<String>,
}

async fn handler_request_add(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(body): Json<RequestAddBody>) -> Result<Json<Value>> {
	let result = mc.exec_request_add(body.request, &library_id, body.media_ref, &user, None).await?;
	Ok(Json(json!(result)))
}

async fn handler_list_processing(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let processings = mc.list_request_processings(&library_id, &user).await?;
	Ok(Json(json!(processings)))
}

async fn handler_get_processing(Path((library_id, id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let processing = mc.get_request_processing(&library_id, &id, &user).await?;
	Ok(Json(json!(processing)))
}

async fn handler_get_processing_progress(Path((library_id, id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let result = mc.get_processing_progress(&library_id, &id, &user).await?;
	Ok(Json(json!(result)))
}

async fn handler_pause_processing(Path((library_id, id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let result = mc.pause_processing(&library_id, &id, &user).await?;
	Ok(Json(json!(result)))
}

async fn handler_remove_processing(Path((library_id, id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	mc.remove_processing(&library_id, &id, &user).await?;
	Ok(Json(json!({"status": "ok"})))
}