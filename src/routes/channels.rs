use axum::{
    body::Body,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use futures::StreamExt;
use http::header::{CONTENT_TYPE, TRANSFER_ENCODING};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::io::ReaderStream;

use crate::{
    domain::channel::Channel,
    model::{
        channels::{ChannelQuery, ImportRequest, StreamQuery},
        users::ConnectedUser,
        ModelController,
    },
    tools::hls_session::PLAYLIST_READY_TIMEOUT_MS,
    Error, Result,
};

use super::ImageRequestOptions;

pub fn routes(mc: ModelController) -> Router {
    Router::new()
        .route("/", get(handler_list))
        .route("/import", post(handler_import))
        .route("/refresh", post(handler_refresh))
        .route("/:id", get(handler_get))
        .route("/:id", delete(handler_delete))
        .route("/:id/image", get(handler_image))
        .route("/:id/tags", post(handler_add_tag))
        .route("/:id/tags/:tagid", delete(handler_remove_tag))
        .route("/:id/stream", get(handler_stream))
        .route("/:id/hls/playlist.m3u8", get(handler_hls_playlist))
        .route("/:id/hls/:segment", get(handler_hls_segment))
        .route("/:id/hls", delete(handler_hls_stop))
        .with_state(mc)
}

async fn handler_list(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<ChannelQuery>,
) -> Result<Json<Vec<Channel>>> {
    let channels = mc.get_channels(&library_id, query, &user).await?;
    Ok(Json(channels))
}

async fn handler_get(
    Path((library_id, channel_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Channel>> {
    let channel = mc.get_channel(&library_id, &channel_id, &user).await?;
    Ok(Json(channel))
}

async fn handler_delete(
    Path((library_id, channel_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    mc.remove_channel(&library_id, &channel_id, &user).await?;
    Ok(Json(json!({"status": "ok"})))
}

async fn handler_image(
    Path((library_id, channel_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<ImageRequestOptions>,
) -> Result<Response> {
    let reader_response = mc
        .channel_image(&library_id, &channel_id, query.kind, query.size, &user)
        .await?;
    let headers = reader_response
        .hearders()
        .map_err(|_| Error::GenericRedseatError)?;
    let stream = ReaderStream::new(reader_response.stream);
    let body = Body::from_stream(stream);
    Ok((headers, body).into_response())
}

async fn handler_import(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(body): Json<ImportRequest>,
) -> Result<Json<Value>> {
    let result = mc.import_m3u(&library_id, body.url, &user).await?;
    Ok(Json(json!(result)))
}

async fn handler_refresh(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    // Refresh is the same as import but without URL override
    let result = mc.import_m3u(&library_id, None, &user).await?;
    Ok(Json(json!(result)))
}

// -- Channel tag management --

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TagBody {
    tag_id: String,
}

async fn handler_add_tag(
    Path((library_id, channel_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(body): Json<TagBody>,
) -> Result<Json<Value>> {
    mc.add_channel_tag(&library_id, &channel_id, &body.tag_id, &user).await?;
    Ok(Json(json!({"status": "ok"})))
}

async fn handler_remove_tag(
    Path((library_id, channel_id, tag_id)): Path<(String, String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    mc.remove_channel_tag(&library_id, &channel_id, &tag_id, &user).await?;
    Ok(Json(json!({"status": "ok"})))
}

// -- MPEG2-TS stream proxy with concurrency guard --

/// Guard that releases the stream slot when the response body is dropped
struct StreamGuard {
    mc: ModelController,
    library_id: String,
    channel_id: String,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        let mc = self.mc.clone();
        let library_id = self.library_id.clone();
        let channel_id = self.channel_id.clone();
        tokio::spawn(async move {
            mc.release_stream_slot(&library_id, &channel_id).await;
        });
    }
}

async fn handler_stream(
    Path((library_id, channel_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<StreamQuery>,
) -> Result<Response> {
    // Enforce concurrent stream limit
    mc.acquire_stream_slot(&library_id, &channel_id).await?;

    let stream_url = match mc
        .get_channel_stream_url(&library_id, &channel_id, query.quality, &user)
        .await
    {
        Ok(url) => url,
        Err(e) => {
            mc.release_stream_slot(&library_id, &channel_id).await;
            return Err(e.into());
        }
    };

    // Proxy the stream through our server
    let client = reqwest::Client::new();
    let upstream = match client.get(&stream_url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            mc.release_stream_slot(&library_id, &channel_id).await;
            return Err(Error::Error(format!("Failed to connect to stream: {}", e)));
        }
    };

    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("video/mp2t")
        .to_string();

    // Wrap the stream with a guard that releases the slot on drop
    let guard = StreamGuard {
        mc: mc.clone(),
        library_id,
        channel_id,
    };

    let byte_stream = upstream.bytes_stream();
    // Map the stream to move the guard's lifetime with it
    let guarded_stream = byte_stream.map(move |chunk| {
        let _guard = &guard;
        chunk.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    });
    let body = Body::from_stream(guarded_stream);

    let response = Response::builder()
        .header(CONTENT_TYPE, content_type)
        .header(TRANSFER_ENCODING, "chunked")
        .body(body)
        .map_err(|e| Error::Error(format!("Failed to build response: {}", e)))?;

    Ok(response)
}

// -- HLS endpoints --

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HlsQuery {
    pub quality: Option<String>,
}

async fn handler_hls_playlist(
    Path((library_id, channel_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<HlsQuery>,
) -> Result<Response> {
    let (_output_dir, playlist_path) = mc
        .get_or_create_hls_session(&library_id, &channel_id, query.quality.clone(), &user)
        .await?;

    // Wait for the playlist file to be created by FFmpeg
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_millis(PLAYLIST_READY_TIMEOUT_MS);

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

    let quality_param = query
        .quality
        .as_deref()
        .map(|q| format!("?quality={}", q))
        .unwrap_or_default();

    // Rewrite segment filenames to proxy URLs
    let rewritten: String = content
        .lines()
        .map(|line| {
            if line.ends_with(".ts") && !line.starts_with('#') {
                format!(
                    "/libraries/{}/channels/{}/hls/{}{}",
                    library_id, channel_id, line, quality_param
                )
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let response = Response::builder()
        .header(CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .body(Body::from(rewritten))
        .map_err(|e| Error::Error(format!("Failed to build response: {}", e)))?;

    Ok(response)
}

async fn handler_hls_segment(
    Path((library_id, channel_id, segment)): Path<(String, String, String)>,
    State(mc): State<ModelController>,
    _user: ConnectedUser,
    Query(query): Query<HlsQuery>,
) -> Result<Response> {
    // Validate segment filename to prevent path traversal
    let is_valid = segment.len() == 13
        && segment.starts_with("seg_")
        && segment.ends_with(".ts")
        && segment[4..9].bytes().all(|b| b.is_ascii_digit());
    if !is_valid {
        return Err(Error::NotFound(format!("Invalid segment: {}", segment)));
    }

    let quality_key = query.quality.as_deref().unwrap_or("best");
    let key = format!("{}:{}:{}", library_id, channel_id, quality_key);

    let (output_dir, session_found) = {
        let sessions = mc.hls_sessions.read().await;
        if let Some(session) = sessions.get(&key) {
            session.touch();
            (session.output_dir.clone(), true)
        } else {
            (std::path::PathBuf::new(), false)
        }
    };

    if !session_found {
        return Err(Error::NotFound("HLS session not found".to_string()));
    }

    let segment_path = output_dir.join(&segment);
    let file = tokio::fs::File::open(&segment_path)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(format!("Segment not found: {}", segment)),
            _ => Error::Error(format!("Failed to open segment: {}", e)),
        })?;

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let response = Response::builder()
        .header(CONTENT_TYPE, "video/mp2t")
        .body(body)
        .map_err(|e| Error::Error(format!("Failed to build response: {}", e)))?;

    Ok(response)
}

async fn handler_hls_stop(
    Path((library_id, channel_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    user.check_library_role(&library_id, crate::domain::library::LibraryRole::Read)?;
    mc.stop_hls_session(&library_id, &channel_id).await?;
    Ok(Json(json!({"status": "ok"})))
}
