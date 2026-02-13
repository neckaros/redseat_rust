use std::io::Cursor;

use crate::{
    domain::library::{LibraryLimits, LibraryRole},
    model::{
        deleted::DeletedQuery,
        libraries::{ServerLibraryForAdd, ServerLibraryForUpdate},
        media_progresses::MediaProgressesQuery,
        media_ratings::MediaRatingsQuery,
        users::ConnectedUser,
        ModelController,
    },
    tools::{
        log::log_info,
        scheduler::{backup::BackupTask, refresh::RefreshTask, RsSchedulerTask},
    },
    Error, Result,
};
use axum::{
    extract::{Multipart, Path, Query, State},
    middleware,
    response::Response,
    routing::{delete, get, patch, post},
    Json, Router,
};
use futures::TryStreamExt;
use hyper::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio_util::io::StreamReader;

use super::mw_auth;

pub fn routes(mc: ModelController) -> Router {
    let delete_routes = Router::new()
        .route("/:id", delete(handler_delete))
        .route_layer(middleware::from_fn_with_state(
            mc.clone(),
            mw_auth::mw_must_be_admin,
        ));

    Router::new()
        .route("/", get(handler_libraries))
        .route("/:id/watermarks", get(handler_watermarks))
        .route("/:id/watermarks/:watermark", get(handler_watermarks_get))
        .route("/:id", get(handler_id))
        .route("/:id", patch(handler_patch))
        .route("/:id/deleted", get(handler_list_deleted))
        .route("/:id/progresses", get(handler_list_progress))
        .route("/:id/ratings", get(handler_list_ratings))
        .route("/", post(handler_post))
        .route("/import", post(handler_import))
        .route("/:id/clean", get(handler_clean))
        .route("/:id/refresh", get(handler_refresh))
        .route("/:id/invitation", post(handler_invitation))
        .merge(delete_routes)
        .with_state(mc)
}

async fn handler_libraries(
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let libraries = mc.get_libraries(&user).await?;
    let body = Json(json!(libraries));
    Ok(body)
}

async fn handler_watermarks(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let libraries = mc.get_watermarks(&library_id, &user).await?;
    let body = Json(json!(libraries));
    Ok(body)
}

async fn handler_watermarks_get(
    Path((library_id, watermark)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Response> {
    let reader = mc.get_watermark(&library_id, &watermark, &user).await?;

    reader
        .into_response(&library_id, None, None, Some((mc.clone(), &user)))
        .await
}

async fn handler_id(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let library = mc.get_library(&library_id, &user).await?;
    if let Some(library) = library {
        let body = Json(json!(library));
        Ok(body)
    } else {
        Err(Error::NotFound(format!(
            "Unable to find library: {}",
            library_id
        )))
    }
}

async fn handler_patch(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(update): Json<ServerLibraryForUpdate>,
) -> Result<Json<Value>> {
    let new_library = mc.update_library(&library_id, update, &user).await?;
    Ok(Json(json!(new_library)))
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LibraryDeleteQuery {
    #[serde(default)]
    delete_media_content: bool,
}

async fn handler_delete(
    Path(library_id): Path<String>,
    Query(query): Query<LibraryDeleteQuery>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<(StatusCode, Json<Value>)> {
    mc.request_remove_library(&library_id, query.delete_media_content, &user)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "accepted": true,
            "libraryId": library_id,
            "deleteMediaContent": query.delete_media_content,
        })),
    ))
}

async fn handler_post(
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(library): Json<ServerLibraryForAdd>,
) -> Result<Json<Value>> {
    let new_library = mc.add_library(library, None, &user).await?;
    Ok(Json(json!(new_library)))
}

async fn handler_import(
    State(mc): State<ModelController>,
    user: ConnectedUser,
    mut multipart: Multipart,
) -> Result<Json<Value>> {
    log_info(
        crate::tools::log::LogServiceType::LibraryCreation,
        "Importing library".to_string(),
    );
    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_string();
        if name != "file" {
            continue;
        }
        //let name = field.name().unwrap().to_string();
        //let filename = field.file_name().unwrap().to_string();
        //let mime: String = field.content_type().unwrap().to_string();
        //let data = field.bytes().await.unwrap();

        let mut reader = StreamReader::new(field.map_err(|multipart_error| {
            std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
        }));

        // Read all bytes from the field into a buffer
        let mut data_vec = Vec::new();
        tokio::io::copy(&mut reader, &mut data_vec).await?;
        // Read first 4 bytes as int32 (little-endian example)
        let size_bytes: [u8; 4] = data_vec[0..4].try_into()?;
        let size = i32::from_le_bytes(size_bytes) as usize;

        // Extract JSON bytes
        let json_bytes = &data_vec[4..4 + size];
        let json_str = std::str::from_utf8(json_bytes).map_err(|_| {
            crate::Error::Error(
                "LibraryImport - Unable to extract utf8 string from information chunk".to_string(),
            )
        })?;
        let library_add: ServerLibraryForAdd = serde_json::from_str(json_str)?;

        // Create reader for remaining data
        let remaining_data = data_vec[4 + size..].to_vec();

        log_info(
            crate::tools::log::LogServiceType::LibraryCreation,
            format!("Importing library: {:?}", library_add),
        );
        log_info(
            crate::tools::log::LogServiceType::LibraryCreation,
            format!("Importing library db size: {}", remaining_data.len()),
        );

        let new_library = mc
            .add_library(library_add, Some(remaining_data), &user)
            .await?;
        return Ok(Json(json!(new_library)));

        //mc.import_library(remaining_reader, library_add, &user).await?;
        //println!("Length of `{}` {}  {} is {} bytes", name, filename, mime, data.len());

        //mc.upload_plugin( reader, &user).await?;
    }

    Ok(Json(json!({"data": "no file found"})))
}

async fn handler_list_deleted(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<DeletedQuery>,
) -> Result<Json<Value>> {
    let deleted = mc.get_deleted(&library_id, query, &user).await?;

    let body = Json(json!(deleted));
    Ok(body)
}

async fn handler_list_ratings(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<MediaRatingsQuery>,
) -> Result<Json<Value>> {
    let deleted = mc.get_medias_ratings(&library_id, query, &user).await?;

    let body = Json(json!(deleted));
    Ok(body)
}

async fn handler_list_progress(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<MediaProgressesQuery>,
) -> Result<Json<Value>> {
    let deleted = mc.get_medias_progresses(&library_id, query, &user).await?;

    let body = Json(json!(deleted));
    Ok(body)
}

async fn handler_clean(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let cleaned = mc.clean_library(&library_id, &user).await?;

    Ok(Json(json!(cleaned)))
}

async fn handler_refresh(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let task = RefreshTask {
        specific_library: Some(library_id),
    };
    tokio::spawn(async move {
        task.execute(mc).await;
    });

    Ok(Json(json!({"started": true})))
}

#[derive(Deserialize)]
struct HandlerInvitationQuery {
    role: LibraryRole,
    limits: LibraryLimits,
}

async fn handler_invitation(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(query): Json<HandlerInvitationQuery>,
) -> Result<Json<Value>> {
    let invitation = mc
        .add_library_invitation(&library_id, vec![query.role.clone()], query.limits, &user)
        .await?;
    Ok(Json(json!(invitation)))
}
