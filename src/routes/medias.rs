use std::{io::Cursor, path::PathBuf, str::FromStr};

use super::{mw_range::RangeDefinition, ImageRequestOptions, ImageUploadOptions};
use crate::{
    domain::{
        media::{
            self, ConvertMessage, ConvertProgress, ItemWithRelations, MediaForUpdate,
            MediaItemReference, MediaWithAction, MediasMessage, VideoMergeRequest,
        },
        ElementAction,
    },
    error::RsError,
    model::{
        self,
        medias::{MediaFileQuery, MediaQuery},
        series::{SerieForUpdate, SerieQuery},
        users::ConnectedUser,
        ModelController,
    },
    plugins::sources::{error::SourcesError, SourceRead},
    tools::{
        log::{log_error, log_info},
        prediction::predict_net,
    },
    Error, Result,
};
use axum::{
    body::Body,
    debug_handler,
    extract::{Multipart, Path, State},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
    Json, Router,
};
use axum_extra::extract::Query;
use futures::TryStreamExt;
use hyper::{header::ACCEPT_RANGES, StatusCode};
use rs_plugin_common_interfaces::{request::{RsGroupDownload, RsRequest}, video::{RsVideoTranscodeStatus, VideoConvertRequest}};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::io::{ReaderStream, StreamReader};

pub fn routes(mc: ModelController) -> Router {
    Router::new()
        .route("/", get(handler_list))
        .route("/count", get(handler_count))
        .route("/loc", get(handler_locs))
        .route("/", delete(handler_multi_delete))
        .route("/", post(handler_post))
        .route("/", patch(handler_multi_patch))
        .route("/exist", get(handler_exist))
        .route("/download", post(handler_download))
        .route("/merge", post(handler_merge))
        .route("/request", post(handler_add_request))
        .route("/transfert/:destination", post(handler_transfert))
        .route("/:id/split", get(handler_split))
        .route("/:id/metadata", get(handler_get))
        .route("/:id/metadata/refresh", get(handler_refresh))
        .route("/:id/sharetoken", get(handler_sharetoken))
        .route("/:id/predict", get(handler_predict))
        .route("/:id/convert", post(handler_convert))
        .route(
            "/:id/convert/plugin/:plugin_id",
            post(handler_convert_plugin),
        )
        .route("/:id/hls", post(handler_media_hls_start))
        .route("/:id/hls", delete(handler_media_hls_stop))
        .route("/:id/hls/playlist.m3u8", get(handler_media_hls_playlist))
        .route("/:id/hls/:segment", get(handler_media_hls_segment))
        .route("/:id", get(handler_get_file))
        .route("/:id/backup/last", get(handler_get_last_backup))
        .route("/:id/backup/:backupid", get(handler_get_backup))
        .route("/:id/backup/metadatas", get(handler_get_backup_medata))
        .route("/:id", patch(handler_patch))
        .route("/:id/progress", patch(handler_patch_progress))
        .route("/:id/rating", patch(handler_patch_rating))
        .route("/:id", delete(handler_delete))
        .route("/:id/image", get(handler_image))
        .route("/:id/image", post(handler_post_image))
        .route("/:id/faces", get(handler_get_media_faces))
        .route("/:id/faces", post(handler_process_media_faces))
        .with_state(mc.clone())
        .nest("/:id/", super::episodes::routes(mc))
}

async fn handler_list(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<MediaQuery>,
) -> Result<Json<Value>> {
    if let Some(filter) = &query.filter {
        let old_query = serde_json::from_str::<MediaQuery>(filter)?;
        //old_query.page_key = query.page_key;
        let libraries = mc.get_medias(&library_id, old_query, &user).await?;
        let body = Json(json!(libraries));
        Ok(body)
    } else {
        let libraries = mc.get_medias(&library_id, query, &user).await?;
        let body = Json(json!(libraries));
        Ok(body)
    }
}

async fn handler_count(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<MediaQuery>,
) -> Result<Json<Value>> {
    if let Some(filter) = &query.filter {
        let old_query = serde_json::from_str::<MediaQuery>(filter)?;
        //old_query.page_key = query.page_key;
        let count = mc.count_medias(&library_id, old_query, &user).await?;
        let body = Json(json!({"count": count}));
        Ok(body)
    } else {
        let count = mc.count_medias(&library_id, query, &user).await?;
        let body = Json(json!({"count": count}));
        Ok(body)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ExistQuery {
    pub hash: String,
}

async fn handler_exist(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<ExistQuery>,
) -> Result<Json<Value>> {
    let media = mc
        .get_media_by_hash(&library_id, query.hash, true, &user)
        .await?;

    let body = Json(json!({"exist": media.is_some(), "media": media}));
    Ok(body)
}

#[derive(Debug, Serialize, Deserialize)]
struct LocQuery {
    pub precision: Option<u32>,
}

async fn handler_locs(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<LocQuery>,
) -> Result<Json<Value>> {
    let libraries = mc.get_locs(&library_id, query.precision, &user).await?;
    let body = Json(json!(libraries));
    Ok(body)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MediasTransfertRequest {
    ids: Vec<String>,
    #[serde(default)]
    delete_original: bool,
}

async fn handler_transfert(
    Path((library_id, destination)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(query): Json<MediasTransfertRequest>,
) -> Result<Json<Value>> {
    let mut new_medias = vec![];
    for id in query.ids {
        let existing = mc.get_media(&library_id, id.clone(), &user).await?.ok_or(
            SourcesError::UnableToFindMedia(
                library_id.to_string(),
                id.to_string(),
                "handler_transfert".to_string(),
            ),
        )?;
        let reader = mc
            .library_file(
                &library_id,
                &id,
                None,
                MediaFileQuery {
                    raw: true,
                    ..Default::default()
                },
                &user,
            )
            .await?
            .into_reader(
                Some(&library_id),
                None,
                None,
                Some((mc.clone(), &user)),
                None,
            )
            .await?;
        let media = mc
            .add_library_file(
                &destination,
                &existing.item.name,
                Some(existing.item.clone().into()),
                reader.stream,
                &user,
            )
            .await?;
        new_medias.push(media)
    }
    let body = Json(json!(new_medias));
    Ok(body)
}

#[derive(Debug, Serialize, Deserialize)]
struct SplitQuery {
    pub from: u32,
    pub to: u32,
}
async fn handler_split(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<SplitQuery>,
) -> Result<Json<Value>> {
    let media = mc
        .split_media(&library_id, media_id, query.from, query.to, &user)
        .await?;
    let body = Json(json!(media));
    Ok(body)
}

async fn handler_get(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let mut library = mc.get_media(&library_id, media_id.clone(), &user).await?;
    if let Some(ref mut media) = library {
        let faces = mc.get_media_faces(&library_id, &media_id, &user).await?;
        media.item.faces = Some(faces);
        let backups = mc
            .get_library_media_backup_files(&library_id, &media_id, &user)
            .await?;
        media.item.backups = Some(backups);
    }
    let body = Json(json!(library));
    Ok(body)
}

async fn handler_refresh(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    mc.update_file_infos(&library_id, &media_id, &user, true)
        .await?;
    mc.process_media(&library_id, &media_id, false, true, &user)
        .await?;
    let media = mc.get_media(&library_id, media_id, &user).await?;
    let body = Json(json!(media));
    Ok(body)
}

async fn handler_sharetoken(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<String> {
    let sharetoken = mc
        .get_file_share_token(&library_id, &media_id, 6 * 60 * 60, &user)
        .await?;
    Ok(sharetoken)
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct PredictOption {
    #[serde(default)]
    pub tag: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct UploadOption {
    #[serde(default)]
    pub spawn: bool,
}

async fn handler_predict(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<PredictOption>,
) -> Result<Json<Value>> {
    let prediction = mc
        .prediction(&library_id, &media_id, query.tag, &user, true)
        .await?;
    let body = Json(json!(prediction));
    //println!("BODY {:?}", body);
    Ok(body)
}

async fn handler_convert(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(query): Json<VideoConvertRequest>,
) -> Result<Json<Value>> {
    mc.convert(&library_id, &media_id, query.clone(), None, &user)
        .await?;

    Ok(Json(json!(query)))
}
async fn handler_convert_plugin(
    Path((library_id, media_id, plugin_id)): Path<(String, String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(query): Json<VideoConvertRequest>,
) -> Result<Json<Value>> {
    mc.convert(
        &library_id,
        &media_id,
        query.clone(),
        Some(plugin_id),
        &user,
    )
    .await?;

    Ok(Json(json!(query)))
}

async fn handler_merge(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(request): Json<VideoMergeRequest>,
) -> Result<Json<Value>> {
    if request.items.len() < 2 {
        return Err(Error::Error("Merge requires at least 2 items".to_string()));
    }

    let response_id = request.id.clone();
    let request_id = request.id.clone();
    let library_id_clone = library_id.clone();
    let mc_clone = mc.clone();
    let user_clone = user.clone();

    tokio::spawn(async move {
        if let Err(e) = mc_clone
            .merge_videos(&library_id_clone, request, &user_clone)
            .await
        {
            log_error(
                crate::tools::log::LogServiceType::Other,
                format!("Merge failed: {:?}", e),
            );
            mc_clone.send_convert_progress(ConvertMessage {
                library: library_id_clone,
                progress: ConvertProgress {
                    id: request_id,
                    filename: String::new(),
                    converted_id: None,
                    done: true,
                    percent: 0.0,
                    status: RsVideoTranscodeStatus::Failed,
                    estimated_remaining_seconds: None,
                    request: None,
                },
            });
        }
    });

    Ok(Json(json!({"id": response_id})))
}

async fn handler_get_file(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    range: Option<RangeDefinition>,
    Query(query): Query<MediaFileQuery>,
) -> Result<Response> {
    let reader = mc
        .library_file(&library_id, &media_id, range.clone(), query, &user)
        .await?;
    reader
        .into_response(&library_id, range, None, Some((mc.clone(), &user)))
        .await
}

async fn handler_get_last_backup(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Response<Body>> {
    let reader = mc
        .get_backup_media(&library_id, &media_id, None, &user)
        .await?;
    let response = reader
        .into_response(&library_id, None, None, Some((mc.clone(), &user)))
        .await?;
    Ok(response)
}

async fn handler_get_backup(
    Path((library_id, media_id, backup_file_id)): Path<(String, String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Response<Body>> {
    let reader = mc
        .get_backup_media(&library_id, &media_id, Some(&backup_file_id), &user)
        .await?;
    let response = reader
        .into_response(&library_id, None, None, Some((mc.clone(), &user)))
        .await?;
    Ok(response)
}

async fn handler_get_backup_medata(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let reader = mc
        .get_library_media_backup_files(&library_id, &media_id, &user)
        .await?;
    Ok(Json(json!(reader)))
}

async fn handler_patch(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(update): Json<MediaForUpdate>,
) -> Result<Json<Value>> {
    let new_credential = mc
        .update_media(&library_id, media_id, update, true, &user)
        .await?;
    Ok(Json(json!(new_credential)))
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct MediaProgressUpdateQuery {
    #[serde(default)]
    pub progress: u64,
}

async fn handler_patch_progress(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    body: Json<MediaProgressUpdateQuery>,
) -> Result<Json<Value>> {
    let new_credential = mc
        .set_media_progress(&library_id, media_id, body.progress, &user)
        .await?;
    Ok(Json(json!(new_credential)))
}

async fn handler_patch_rating(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(update): Json<MediaForUpdate>,
) -> Result<Json<Value>> {
    let new_credential = mc
        .update_media(&library_id, media_id, update, true, &user)
        .await?;
    Ok(Json(json!(new_credential)))
}

async fn handler_delete(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let library = mc.remove_media(&library_id, &media_id, &user).await?;
    let body = Json(json!(library));
    Ok(body)
}

async fn handler_post(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    mut multipart: Multipart,
) -> Result<Json<Value>> {
    let mut info: MediaForUpdate = MediaForUpdate::default();
    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_string();
        if name == "info" {
            let text = &field.text().await?;
            info = serde_json::from_str(&text)?;
        } else if name == "file" {
            let filename = field.file_name().unwrap().to_string();
            let mime = if info.mimetype.is_none() {
                field.content_type().map(|c| c.to_owned())
            } else {
                info.mimetype
            };
            let size = field
                .headers()
                .get("Content-Length")
                .and_then(|len| len.to_str().ok())
                .and_then(|len_str| len_str.parse::<u64>().ok());

            info.name = info.name.or(Some(filename.clone()));
            info.mimetype = mime;
            info.size = size;

            let reader = StreamReader::new(field.map_err(|multipart_error| {
                std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
            }));

            let media = mc
                .add_library_file(&library_id, &filename, Some(info), reader, &user)
                .await?;
            return Ok(Json(json!(media)));
        }
    }
    Err(Error::Error("No media provided".to_owned()))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct MediasRemoveRequest {
    ids: Vec<String>,
}

async fn handler_multi_delete(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    requesting_user: ConnectedUser,
    Json(updates): Json<MediasRemoveRequest>,
) -> Result<Json<Value>> {
    let mut removed = vec![];
    for id in updates.ids {
        let removed_media = mc.remove_media(&library_id, &id, &requesting_user).await;
        if let Err(RsError::Model(model::error::Error::MediaNotFound(media_id))) = removed_media {
            log_info(
                crate::tools::log::LogServiceType::Other,
                format!("Media id {} not delete (not found)", media_id),
            );
        } else if let Err(error) = removed_media {
            log_error(
                crate::tools::log::LogServiceType::Other,
                format!("Media id {} not delete (error: {:?})", id, error),
            );
        } else if let Ok(media) = removed_media {
            removed.push(media);
        }
    }
    mc.send_media(MediasMessage {
        library: library_id.to_string(),
        medias: removed
            .iter()
            .map(|m| MediaWithAction {
                media: ItemWithRelations { item: m.clone(), relations: None },
                action: ElementAction::Deleted,
            })
            .collect(),
    });
    Ok(Json(json!(removed)))
}
#[derive(Debug, Serialize, Deserialize, Clone)]
struct MediasUpdateRequest {
    ids: Vec<String>,
    update: MediaForUpdate,
}

async fn handler_multi_patch(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    requesting_user: ConnectedUser,
    Json(updates): Json<MediasUpdateRequest>,
) -> Result<Json<Value>> {
    let mut updated = vec![];
    for id in updates.ids {
        updated.push(
            mc.update_media(
                &library_id,
                id,
                updates.update.clone(),
                true,
                &requesting_user,
            )
            .await?,
        );
    }
    mc.send_media(MediasMessage {
        library: library_id.to_string(),
        medias: updated
            .iter()
            .map(|m| MediaWithAction {
                media: ItemWithRelations { item: m.clone(), relations: None },
                action: ElementAction::Updated,
            })
            .collect(),
    });

    Ok(Json(json!(updated)))
}

async fn handler_download(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<UploadOption>,
    Json(download): Json<RsGroupDownload>,
) -> Result<Json<Value>> {
    if query.spawn {
        tokio::spawn(async move {
            let _ = mc
                .download_library_url(&library_id, download, &user)
                .await
                .expect("Unable to download");
        });

        Ok(Json(json!({"downloading": true})))
    } else {
        match mc
            .download_library_url(&library_id, download, &user)
            .await
        {
            Ok(body) => Ok(Json(json!(body))),
            Err(crate::Error::Model(crate::model::error::Error::NeedFileSelection(request))) => {
                Ok(Json(json!({
                    "needFileSelection": true,
                    "request": request
                })))
            }
            Err(e) => Err(e),
        }
    }
}

async fn handler_add_request(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(request): Json<RsRequest>,
) -> Result<Json<Value>> {
    let group_thumbnail_url = request.thumbnail_url.clone();
    let group_filename = request.filename.clone();
    let group_mime = request.mime.clone();

    let group = RsGroupDownload {
        requests: vec![request],
        group: false,
        group_thumbnail_url,
        group_filename,
        group_mime,
        ..Default::default()
    };
    let added = mc
        .download_library_url(&library_id, group, &user)
        .await?;
    let added = added.into_iter().next().ok_or(Error::Error("No media added".to_string()))?;

    Ok(Json(json!(added)))
}

async fn handler_image(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<ImageRequestOptions>,
) -> Result<Response> {
    let reader_response = mc
        .media_image(&library_id, &media_id, query.size, &user)
        .await?;

    let headers = reader_response
        .hearders()
        .map_err(|_| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);

    Ok((headers, body).into_response())
}

#[debug_handler]
async fn handler_post_image(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    mut multipart: Multipart,
) -> Result<Json<Value>> {
    while let Some(field) = multipart.next_field().await.unwrap() {
        let mut reader = StreamReader::new(field.map_err(|multipart_error| {
            std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
        }));

        // Read all bytes from the field into a buffer
        let mut data = Vec::new();
        tokio::io::copy(&mut reader, &mut data).await?;
        let reader = Box::pin(Cursor::new(data));
        mc.update_media_image(&library_id, &media_id, Box::pin(reader), &user)
            .await?;
    }

    Ok(Json(json!({"data": "ok"})))
}

async fn handler_get_media_faces(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let faces = mc.get_media_faces(&library_id, &media_id, &user).await?;
    Ok(Json(json!(faces)))
}

async fn handler_process_media_faces(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    // Get all existing faces for this media (both recognized and unrecognized)
    let existing_faces = mc.get_media_faces(&library_id, &media_id, &user).await?;

    // Delete each existing face (best effort - continue even if some fail)
    for face in &existing_faces {
        if let Err(e) = mc.delete_face(&library_id, &face.id, &user).await {
            log_error(
                crate::tools::log::LogServiceType::Other,
                format!(
                    "Failed to delete face {} for media {}: {}",
                    face.id, media_id, e
                ),
            );
        }
    }

    // Start new face recognition process
    let detected_faces = mc
        .process_media_faces(&library_id, &media_id, &user, None)
        .await?;

    Ok(Json(json!(detected_faces)))
}

// -- Media HLS handlers --

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MediaHlsStartRequest {
    convert: Option<VideoConvertRequest>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MediaHlsQuery {
    /// Session key returned by the start endpoint, used to look up the right session
    /// when multiple transcode variants exist for the same media
    key: Option<String>,
}

/// Look up a media HLS session by exact key or by library:media prefix.
/// Touches the session to reset inactivity timeout.
fn find_media_hls_session<'a>(
    sessions: &'a std::collections::HashMap<String, crate::tools::media_hls_session::MediaHlsSession>,
    query_key: &Option<String>,
    library_id: &str,
    media_id: &str,
) -> Option<&'a crate::tools::media_hls_session::MediaHlsSession> {
    let session = if let Some(ref session_key) = query_key {
        sessions.get(session_key)
    } else {
        let prefix = format!("{}:{}:", library_id, media_id);
        sessions.values().find(|s| s.key.starts_with(&prefix))
    };
    if let Some(s) = session {
        s.touch();
    }
    session
}

async fn handler_media_hls_start(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(body): Json<MediaHlsStartRequest>,
) -> Result<Json<Value>> {
    let key = mc
        .get_or_create_media_hls_session(&library_id, &media_id, body.convert, &user)
        .await?;

    Ok(Json(json!({
        "key": key,
        "playlistUrl": format!("/libraries/{}/medias/{}/hls/playlist.m3u8?key={}", library_id, media_id, key)
    })))
}

async fn handler_media_hls_playlist(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    Query(query): Query<MediaHlsQuery>,
) -> Result<Response> {
    use http::header::{CACHE_CONTROL, CONTENT_TYPE};
    use crate::tools::media_hls_session::MEDIA_PLAYLIST_READY_TIMEOUT_MS;

    // Find the session
    let (playlist_path, key) = {
        let sessions = mc.media_hls_sessions.read().await;
        let session = find_media_hls_session(&sessions, &query.key, &library_id, &media_id)
            .ok_or_else(|| Error::NotFound("Media HLS session not found".to_string()))?;
        (session.playlist_path.clone(), session.key.clone())
    };

    // Wait for the playlist file to be created by FFmpeg
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_millis(MEDIA_PLAYLIST_READY_TIMEOUT_MS);

    loop {
        if let Ok(meta) = tokio::fs::metadata(&playlist_path).await {
            if meta.len() > 0 {
                break;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Error(
                "Timed out waiting for HLS playlist to be ready".to_string(),
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    // Read and rewrite the playlist
    let content = tokio::fs::read_to_string(&playlist_path)
        .await
        .map_err(|e| Error::Error(format!("Failed to read HLS playlist: {}", e)))?;

    // Rewrite segment filenames to proxy URLs
    let rewritten: String = content
        .lines()
        .map(|line| {
            if line.ends_with(".ts") && !line.starts_with('#') {
                format!(
                    "/libraries/{}/medias/{}/hls/{}?key={}",
                    library_id, media_id, line, key
                )
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let response = Response::builder()
        .header(CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .header(CACHE_CONTROL, "no-cache, no-store")
        .body(Body::from(rewritten))
        .map_err(|e| Error::Error(format!("Failed to build response: {}", e)))?;

    Ok(response)
}

async fn handler_media_hls_segment(
    Path((library_id, media_id, segment)): Path<(String, String, String)>,
    State(mc): State<ModelController>,
    Query(query): Query<MediaHlsQuery>,
) -> Result<Response> {
    use http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE};

    // Validate segment filename to prevent path traversal
    let is_valid = segment.len() == 12
        && segment.starts_with("seg_")
        && segment.ends_with(".ts")
        && segment[4..9].bytes().all(|b| b.is_ascii_digit());
    if !is_valid {
        return Err(Error::NotFound(format!("Invalid segment: {}", segment)));
    }

    let output_dir = {
        let sessions = mc.media_hls_sessions.read().await;
        let session = find_media_hls_session(&sessions, &query.key, &library_id, &media_id)
            .ok_or_else(|| Error::NotFound("Media HLS session not found".to_string()))?;
        session.output_dir.clone()
    };

    let segment_path = output_dir.join(&segment);
    let file = tokio::fs::File::open(&segment_path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                Error::NotFound(format!("Segment not found: {}", segment))
            }
            _ => Error::Error(format!("Failed to open segment: {}", e)),
        })?;

    let file_size = file.metadata().await.map(|m| m.len()).unwrap_or(0);

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    // VOD segments are immutable — cache for 1 hour
    let mut response_builder = Response::builder()
        .header(CONTENT_TYPE, "video/mp2t")
        .header(CACHE_CONTROL, "public, max-age=3600");
    if file_size > 0 {
        response_builder = response_builder.header(CONTENT_LENGTH, file_size);
    }
    let response = response_builder
        .body(body)
        .map_err(|e| Error::Error(format!("Failed to build response: {}", e)))?;

    Ok(response)
}

async fn handler_media_hls_stop(
    Path((library_id, media_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
    mc.stop_media_hls_session(&library_id, &media_id).await?;
    Ok(Json(json!({"status": "ok"})))
}
