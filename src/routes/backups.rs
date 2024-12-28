
use crate::{domain::backup::BackupWithStatus, error::RsError, model::{backups::{BackupForAdd, BackupForUpdate}, users::ConnectedUser, ModelController}, tools::scheduler::{backup::BackupTask, RsSchedulerTask}, Result};
use axum::{body::Body, extract::{Path, State}, response::Response, routing::{delete, get, patch, post}, Json, Router};
use serde_json::{json, Value};



pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/", post(handler_post))
		.route("/:id", get(handler_get))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))

		
		.route("/:id/medias/:media_id", get(handler_get_last_backup_media))

		.route("/:id/start", get(handler_backup))
		.with_state(mc)
        
}

async fn handler_list(State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let backups = mc.get_backups_with_status(&user).await?;
	let body = Json(json!(backups));
	Ok(body)
}

async fn handler_get(Path(backup_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let backup = mc.get_backup_with_status(&backup_id, &user).await?.ok_or(RsError::NotFound)?;
	let body = Json(json!(backup));
	Ok(body)
}

async fn handler_patch(Path(backup_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<BackupForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_backup(&backup_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path(backup_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_backup(&backup_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_post(State(mc): State<ModelController>, user: ConnectedUser, Json(backup): Json<BackupForAdd>) -> Result<Json<Value>> {
	let credential = mc.add_backup(backup, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}

async fn handler_get_last_backup_media(Path((backup_id, media_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Response<Body>> {
	let backups = mc.get_backup_media_backup_files(&backup_id, &media_id, &user).await?;
	let last = backups.last().ok_or(RsError::NotFound)?;
	let reader = mc.get_backup_file_reader(&last.id, &user).await?;
	let response = reader.into_response("nope", None, None, Some((mc.clone(), &user))).await?;
	Ok(response)
}


async fn handler_backup(Path(backup_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	tokio::spawn(async move {
		let backup_task = BackupTask {
			specific_backup: Some(backup_id),
		};
		let process = backup_task.execute(mc).await;

		match process {
			Ok(_) => println!("Task completed successfully"),
			Err(e) => println!("Task error: {}", e),
		}

		Ok::<_, RsError>(())
	});
	Ok(Json(json!({"data": "ok"})))
}
