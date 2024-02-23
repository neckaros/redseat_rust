
use crate::{model::{libraries::ServerLibraryForUpdate, users::ConnectedUser, ModelController}, Error, Result};
use axum::{extract::{Path, State}, routing::{get, patch}, Extension, Json, Router};
use serde_json::{json, Value};
use socketioxide::SocketIo;



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_libraries))
		.route("/:id", get(handler_id))
		.route("/:id", patch(handler_patch))
		.with_state(mc)
        
}

async fn handler_libraries(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.get_libraries(&user).await?;
	let body = Json(json!(libraries));
	mc.send_watched();
	Ok(body)
}

async fn handler_id(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_library(&library_id, &user).await?;
	if let Some(library) = library {
		let body = Json(json!(library));
		Ok(body)
	} else {
		Err(Error::NotFound)
	}
}

async fn handler_patch(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(payload): Json<ServerLibraryForUpdate>) -> Result<Json<Value>> {
	Ok(Json(json!(payload)))
}