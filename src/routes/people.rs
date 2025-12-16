
use crate::{model::{people::{PeopleQuery, PersonForAdd, PersonForUpdate}, users::ConnectedUser, ModelController}, Result};
use axum::{body::Body, debug_handler, extract::{Multipart, Path, Query, State}, response::{IntoResponse, Response}, routing::{delete, get, patch, post}, Json, Router};
use futures::TryStreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::io::{ReaderStream, StreamReader};
use crate::Error;

use super::ImageRequestOptions;


pub fn routes(mc: ModelController) -> Router {
	Router::new()
		.route("/", get(handler_list))
		.route("/", post(handler_post))
        .route("/detect-faces", post(handler_detect_faces_in_media))
        .route("/cluster-faces", post(handler_cluster_unassigned_faces))
        .route("/unassigned-faces", get(handler_get_unassigned_faces))
        .route("/batch-detect", post(handler_batch_detect_faces))
        .route("/merge", post(handler_merge_people))
        .route("/assign-face", post(handler_assign_face_to_person))
        .route("/unassign-face", post(handler_unassign_face_from_person))
        .route("/tasks/face-recognition", post(handler_start_face_recognition_task))
        
		.route("/:id", get(handler_get))
		.route("/:id", patch(handler_patch))
		.route("/:id", delete(handler_delete))
		.route("/:id/image", get(handler_image))
		.route("/:id/image", post(handler_post_image))
        .route("/:id/faces", get(handler_get_person_faces))
        .route("/faces/:face_id", delete(handler_delete_face))
        .route("/faces/:face_id/image", get(handler_get_face_image))
		.with_state(mc)
        
}

async fn handler_list(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<PeopleQuery>) -> Result<Json<Value>> {
	let libraries = mc.get_people(&library_id, query, &user).await?;
	let body = Json(json!(libraries));
	Ok(body)
}

async fn handler_get(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.get_person(&library_id, tag_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

async fn handler_patch(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Json(update): Json<PersonForUpdate>) -> Result<Json<Value>> {
	let new_credential = mc.update_person(&library_id, tag_id, update, &user).await?;
	Ok(Json(json!(new_credential)))
}

async fn handler_delete(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser) -> Result<Json<Value>> {
	let library = mc.remove_person(&library_id, &tag_id, &user).await?;
	let body = Json(json!(library));
	Ok(body)
}

#[debug_handler]
async fn handler_image(Path((library_id, tag_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, Query(query): Query<ImageRequestOptions>) -> Result<Response> {
	let reader_response = mc.person_image(&library_id, &tag_id, query.kind, query.size, &user).await?;

	let headers = reader_response.hearders().map_err(|_| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);
	
    Ok((headers, body).into_response())
}

async fn handler_post_image(Path((library_id, person_id)): Path<(String, String)>, State(mc): State<ModelController>, user: ConnectedUser, mut multipart: Multipart) -> Result<Json<Value>> {
	while let Some(field) = multipart.next_field().await.unwrap() {
		let reader = StreamReader::new(field.map_err(|multipart_error| {
			std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
		}));

		mc.update_person_image(&library_id, &person_id, &None, reader, &user).await?;
    }
	
    Ok(Json(json!({"data": "ok"})))
}

async fn handler_post(Path(library_id): Path<String>, State(mc): State<ModelController>, user: ConnectedUser, Json(tag): Json<PersonForAdd>) -> Result<Json<Value>> {
	let credential = mc.add_pesron(&library_id, tag, &user).await?;
	let body = Json(json!(credential));
	Ok(body)
}

// FACE RECOGNITION

#[derive(Deserialize)]
struct DetectFacesRequest {
    media_ids: Vec<String>,
}

async fn handler_detect_faces_in_media(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(payload): Json<DetectFacesRequest>
) -> Result<Json<Value>> {
    let mut results = Vec::new();
    for media_id in payload.media_ids {
        let res = mc.process_media_faces(&library_id, &media_id, &user).await?;
        results.push(json!({ "media_id": media_id, "faces": res }));
    }
    Ok(Json(json!(results)))
}

async fn handler_cluster_unassigned_faces(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser
) -> Result<Json<Value>> {
    let result = mc.cluster_unassigned_faces(&library_id, &user).await?;
    Ok(Json(json!(result)))
}

async fn handler_get_unassigned_faces(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<UnassignedFacesQuery>
) -> Result<Json<Value>> {
    // Always provide a limit for API calls (default 50)
    let limit = query.limit.or(Some(50));
    let faces = mc.get_all_unassigned_faces(&library_id, limit, query.created_before, &user).await?;
    Ok(Json(json!(faces)))
}

async fn handler_batch_detect_faces(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(payload): Json<DetectFacesRequest>
) -> Result<Json<Value>> {
    let mut results = Vec::new();
    for media_id in payload.media_ids {
        match mc.process_media_faces(&library_id, &media_id, &user).await {
            Ok(faces) => {
                results.push(json!({
                    "media_id": media_id,
                    "status": "success",
                    "faces": faces
                }));
            }
            Err(e) => {
                results.push(json!({
                    "media_id": media_id,
                    "status": "error",
                    "error": e.to_string()
                }));
            }
        }
    }
    Ok(Json(json!(results)))
}

async fn handler_get_person_faces(
    Path((library_id, person_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser
) -> Result<Json<Value>> {
    let faces = mc.get_person_faces(&library_id, &person_id, &user).await?;
    Ok(Json(json!(faces)))
}

async fn handler_delete_face(
    Path((library_id, face_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser
) -> Result<Json<Value>> {
    mc.delete_face(&library_id, &face_id, &user).await?;
    Ok(Json(json!({"status": "deleted"})))
}

async fn handler_get_face_image(
    Path((library_id, face_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser
) -> Result<Response> {
    let image_bytes = mc.get_face_image(&library_id, &face_id, &user).await?;
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(axum::http::header::CONTENT_TYPE, "image/avif".parse().unwrap());
    Ok((headers, Body::from(image_bytes)).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergePeopleRequest {
    source_person_id: String,
    target_person_id: String,
}

async fn handler_merge_people(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(payload): Json<MergePeopleRequest>
) -> Result<Json<Value>> {
    let faces_transferred = mc.merge_people(&library_id, &payload.source_person_id, &payload.target_person_id, &user).await?;
    Ok(Json(json!({
        "status": "merged",
        "faces_transferred": faces_transferred
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssignFaceRequest {
    face_ids: Vec<String>,
    person_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnassignFaceRequest {
    face_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnassignedFacesQuery {
    limit: Option<usize>,
    created_before: Option<i64>,
}

async fn handler_assign_face_to_person(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(payload): Json<AssignFaceRequest>
) -> Result<Json<Value>> {
    // First unassign any faces that are already assigned to a person
    let unassigned_count = mc.unassign_faces_from_person(&library_id, &payload.face_ids, &user).await?;
    
    // Then assign all faces to the new person
    let assigned_count = mc.assign_unassigned_faces_to_person(&library_id, &payload.face_ids, &payload.person_id, &user).await?;
    
    Ok(Json(json!({
        "status": "assigned",
        "face_ids": payload.face_ids,
        "person_id": payload.person_id,
        "unassigned_count": unassigned_count,
        "assigned_count": assigned_count
    })))
}

async fn handler_unassign_face_from_person(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(payload): Json<UnassignFaceRequest>
) -> Result<Json<Value>> {
    let unassigned_count = mc.unassign_faces_from_person(&library_id, &payload.face_ids, &user).await?;
    Ok(Json(json!({
        "status": "unassigned",
        "face_ids": payload.face_ids,
        "unassigned_count": unassigned_count
    })))
}

async fn handler_start_face_recognition_task(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser
) -> Result<Json<Value>> {
    use crate::tools::scheduler::{face_recognition::FaceRecognitionTask, RsSchedulerWhen, RsTaskType};
    
    let task = FaceRecognitionTask {
        specific_library: Some(library_id.clone())
    };
    
    mc.scheduler.add(RsTaskType::Face, RsSchedulerWhen::At(0), task).await?;
    
    Ok(Json(json!({
        "status": "started",
        "message": "Face recognition task has been queued to start immediately"
    })))
}
