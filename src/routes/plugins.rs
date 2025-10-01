
use std::io::Cursor;

use crate::{domain::{media::{MediaForUpdate, DEFAULT_MIME}, plugin::{PluginForAdd, PluginForInstall, PluginForUpdate, PluginRepoAdd}}, error::RsError, model::{credentials::CredentialForAdd, plugins::PluginQuery, users::ConnectedUser}, tools::{array_tools::value_to_hashmap, convert::{convert_from_to, ConvertFileSource}, http_tools::download_latest_wasm}, ModelController, Result};
use axum::{extract::{Multipart, Path, Query, State}, routing::{delete, get, patch, post}, Json, Router};
use futures::TryStreamExt;
use rs_plugin_common_interfaces::{request::RsRequest, url::RsLink, CredentialType, PluginType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::io::StreamReader;
use nanoid::nanoid;


pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/upload", post(handler_upload_plugin))
		.route("/upload/repo", post(handler_upload_repo_plugin))
		.route("/install", post(handler_install))

		.route("/reload", get(handler_reload_plugins))

		.route("/parse", get(handler_parse))
		.route("/expand", post(handler_expand))
		.route("/convert", post(handler_convert))

		.route("/urlrequest", get(handler_urlrequest))
		.route("/", post(handler_post))
		.route("/:id", get(handler_get))
		.route("/:id/reload", get(handler_reload))
		.route("/:id/reporefresh", get(handler_refresh_repo))
		.route("/:id/oauthtoken", post(handler_exchange_token))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))
		.route("/:id/file", delete(handler_delete_file))
		.with_state(mc)
        
}

async fn handler_list(State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<PluginQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_all_plugins(query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_install(State(mc): State<ModelController>, user: ConnectedUser, Json(plugin): Json<PluginForInstall>) -> Result<Json<Value>> {
	let libraries = mc.install_plugin(plugin, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_upload_plugin(State(mc): State<ModelController>, user: ConnectedUser
, mut multipart: Multipart) -> Result<Json<Value>> {
	while let Some(field) = multipart.next_field().await.unwrap() {
        //let name = field.name().unwrap().to_string();
		//let filename = field.file_name().unwrap().to_string();
		//let mime: String = field.content_type().unwrap().to_string();
        //let data = field.bytes().await.unwrap();

		let mut reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		
		// Read all bytes from the field into a buffer
		let mut data = Vec::new();
		tokio::io::copy(&mut reader, &mut data).await?;
		let reader = Box::pin(Cursor::new(data));
		
        //println!("Length of `{}` {}  {} is {} bytes", name, filename, mime, data.len());
			mc.upload_plugin( reader, &user).await?;
    }
	
    Ok(Json(json!({"data": "ok"})))
}

async fn handler_upload_repo_plugin(State(mc): State<ModelController>, user: ConnectedUser, Json(plugin): Json<PluginRepoAdd>) -> Result<Json<Value>> {
	

	let path = mc.upload_repo_plugin(&plugin.url, &user).await?;
    Ok(Json(json!({"path": path})))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ExpandQuery {
	pub url: String,
}

async fn handler_reload_plugins(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	mc.reload_plugins(&user).await?;
	let body = Json(json!({
		"status": "OK"
	}));
	Ok(body)
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

async fn handler_refresh_repo(Path(plugin_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let update_path = mc.refresh_repo_plugin(&plugin_id, &user).await?;
	let body = Json(json!(json!({
		"path": update_path
	})));
	Ok(body)
}

async fn handler_exchange_token(Path(plugin_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(payload): Json<Value>) -> Result<()> {
	let params = value_to_hashmap(payload)?;
	let name = params.get("name").cloned().unwrap_or("Credential".to_string());
	let created_credential = mc.exec_token_exchange(&plugin_id, params, &user).await?;
	let credential_for_add = CredentialForAdd {
    name: name,
    source: plugin_id,
    kind: CredentialType::Oauth { url: "None".to_string() },
    login: Some("token".to_string()),
    password: created_credential.password,
    settings: created_credential.settings,
    user_ref: user.user_id().ok(),
    refresh_token: created_credential.refresh_token,
    expires: created_credential.expires,
};
	let credential = mc.add_credential(credential_for_add, &user).await?;
	Ok(())
}


async fn handler_patch(Path(plugin_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<PluginForUpdate>) -> Result<(Json<Value>)> {
	let new_credential = mc.update_plugin(&plugin_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path(plugin_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_plugin(&plugin_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_delete_file(Path(plugin_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_plugin_wasm(&plugin_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(State(mc): State<ModelController>, user: ConnectedUser, Json(plugin): Json<PluginForAdd>) -> Result<Json<Value>> {
	let credential = mc.add_plugin(plugin, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct ConvertParams {
	pub from: Option<String>,
	pub to: String,
}

async fn handler_convert(State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ConvertParams>, mut multipart: Multipart) -> Result<Vec<u8>> {
	let mut info:MediaForUpdate = MediaForUpdate::default();
	while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap_or(&nanoid!()).to_string();
		if name == "info" {
			let text = &field.text().await?;
			info = serde_json::from_str(&text)?;
		} else if name == "file" {
			let filename = field.file_name().unwrap().to_string();
			let mime = if query.from.is_none() { field.content_type().map(|c| c.to_owned()) } else { query.from.clone() };
			let size = field.headers().get("Content-Length")
            .and_then(|len| len.to_str().ok())
            .and_then(|len_str| len_str.parse::<u64>().ok());
        	
			println!("Expected file length: {:?}", size);

			info.name = info.name.or(Some(filename.clone()));
			info.mimetype = mime.clone();
			info.size = size;
			
			let reader = StreamReader::new(field.map_err(|multipart_error| {
				std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
			}));

			let source = ConvertFileSource { 
				mime: mime.unwrap_or(DEFAULT_MIME.to_string()),
				reader
			};
			
			let result = convert_from_to(source, &query.to).await?;
			return Ok(result)
		}
    }
	Err(RsError::Error("No media provided".to_owned()))
}