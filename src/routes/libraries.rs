
use crate::{domain::library::LibraryRole, model::{libraries::{ServerLibraryForAdd, ServerLibraryForUpdate}, users::{ConnectedUser, UserRole}, ModelController}, Error, Result};
use axum::{extract::{Path, State}, routing::{delete, get, patch, post}, Json, Router};
use hyper::StatusCode;
use serde_json::{json, Value};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_libraries))
		.route("/:id", get(handler_id))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))
		.route("/", post(handler_post))
		.with_state(mc)
        
}

async fn handler_libraries(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	user.check_role(&UserRole::Read)?;
	let libraries = mc.get_libraries(&user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_id(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	user.check_library_role(&library_id, &LibraryRole::Read)?;
	let library = mc.get_library(&library_id, &user).await?;
	if let Some(library) = library {
		let body = Json(json!(library));
		Ok(body)
	} else {
		Err(Error::NotFound)
	}
}

async fn handler_patch(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<ServerLibraryForUpdate>) -> Result<Json<Value>> {
	user.check_library_role(&library_id, &LibraryRole::Admin)?;
	let new_library = mc.update_library(&library_id, update, &user).await?;
	Ok(Json(json!(new_library)))
}

async fn handler_delete(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<StatusCode> {
	user.check_library_role(&library_id, &LibraryRole::Admin)?;
	mc.remove_library( &library_id, &user).await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn handler_post(State(mc): State<ModelController>, user: ConnectedUser, Json(library): Json<ServerLibraryForAdd>) -> Result<Json<Value>> {
	user.check_role(&UserRole::Admin)?;
	let new_library = mc.add_library( library, &user).await?;
	Ok(Json(json!(new_library)))
}


