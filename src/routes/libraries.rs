
use crate::{domain::library::{LibraryLimits, LibraryRole}, model::{deleted::DeletedQuery, libraries::{ServerLibraryForAdd, ServerLibraryForUpdate}, media_progresses::MediaProgressesQuery, media_ratings::MediaRatingsQuery, users::ConnectedUser, ModelController}, tools::scheduler::{backup::BackupTask, refresh::RefreshTask, RsSchedulerTask}, Error, Result};
use axum::{extract::{Path, Query, State}, response::Response, routing::{delete, get, patch, post}, Json, Router};
use hyper::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_libraries))
		.route("/:id/watermarks", get(handler_watermarks))
		.route("/:id/watermarks/:watermark", get(handler_watermarks_get))
		.route("/:id", get(handler_id))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))
		.route("/:id/deleted", get(handler_list_deleted))
		.route("/:id/progresses", get(handler_list_progress))
		.route("/:id/ratings", get(handler_list_ratings))
		.route("/", post(handler_post))
		
		.route("/:id/clean", get(handler_clean))
		.route("/:id/refresh", get(handler_refresh))

		.route("/:id/invitation", post(handler_invitation))
		.with_state(mc)
        
}

async fn handler_libraries(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.get_libraries(&user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_watermarks(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let libraries = mc.get_watermarks(&library_id, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_watermarks_get(Path((library_id, watermark)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Response> {
	let reader = mc.get_watermark(&library_id, &watermark, &user).await?;
	
	reader.into_response(&library_id, None, None, Some((mc.clone(), &user))).await
}


async fn handler_id(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_library(&library_id, &user).await?;
	if let Some(library) = library {
		let body = Json(json!(library));
		Ok(body)
	} else {
		Err(Error::NotFound(format!("Unable to find library: {}", library_id)))
	}
}

async fn handler_patch(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<ServerLibraryForUpdate>) -> Result<Json<Value>> {
	let new_library = mc.update_library(&library_id, update, &user).await?;
	Ok(Json(json!(new_library)))
}

async fn handler_delete(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<StatusCode> {
	mc.remove_library( &library_id, &user).await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn handler_post(State(mc): State<ModelController>, user: ConnectedUser, Json(library): Json<ServerLibraryForAdd>) -> Result<Json<Value>> {
	let new_library = mc.add_library( library, &user).await?;
	Ok(Json(json!(new_library)))
}

async fn handler_list_deleted(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<DeletedQuery>) -> Result<Json<Value>> {
	let deleted = mc.get_deleted(&library_id, query, &user).await?;

	let body = Json(json!(deleted));
	Ok(body)
	
}

async fn handler_list_ratings(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<MediaRatingsQuery>) -> Result<Json<Value>> {
	let deleted = mc.get_medias_ratings(&library_id, query, &user).await?;

	let body = Json(json!(deleted));
	Ok(body)
	
}

async fn handler_list_progress(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<MediaProgressesQuery>) -> Result<Json<Value>> {
	let deleted = mc.get_medias_progresses(&library_id, query, &user).await?;

	let body = Json(json!(deleted));
	Ok(body)
	
}


async fn handler_clean(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let cleaned = mc.clean_library(&library_id, &user).await?;
		
	Ok(Json(json!(cleaned)))
}

async fn handler_refresh(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let task = RefreshTask {
		specific_library: Some(library_id)
	};
	tokio::spawn(async move {
		task.execute(mc).await;
	});
		
	Ok(Json(json!({"started": true})))
}

#[derive(Deserialize)]
struct HandlerInvitationQuery {
    role: LibraryRole,
	limits: LibraryLimits
}

async fn handler_invitation(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(query): Json<HandlerInvitationQuery>) -> Result<Json<Value>> {
	let invitation = mc.add_library_invitation(&library_id, vec![query.role.clone()], query.limits, &user).await?;
	Ok(Json(json!(invitation)))
}