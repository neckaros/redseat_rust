use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use futures::StreamExt;
use http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{io, time::Duration};
use tokio_util::io::ReaderStream;

use crate::{
    domain::channel::Channel,
    model::{
        channels::{ChannelQuery, ImportRequest, StreamQuery},
        users::ConnectedUser,
        ModelController,
    },
    tools::{
        hls_session::PLAYLIST_READY_TIMEOUT_MS,
        log::{log_error, log_info, LogServiceType},
    },
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
    mc.add_channel_tag(&library_id, &channel_id, &body.tag_id, &user)
        .await?;
    Ok(Json(json!({"status": "ok"})))
}

async fn handler_remove_tag(
    Path((library_id, channel_id, tag_id)): Path<(String, String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    mc.remove_channel_tag(&library_id, &channel_id, &tag_id, &user)
        .await?;
    Ok(Json(json!({"status": "ok"})))
}

// -- MPEG2-TS stream proxy with concurrency guard --

const MAX_UPSTREAM_RECONNECT_ATTEMPTS: u32 = 8;
const STABLE_UPSTREAM_BYTES: usize = 512 * 1024;
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const UPSTREAM_READ_TIMEOUT: Duration = Duration::from_secs(15);
/// After a stream was closed for this one, the provider may still count its
/// connection for a moment: the first connection is tried again this long.
const REPLACED_UPSTREAM_CONNECT_WINDOW: Duration = Duration::from_secs(6);
/// Chunks kept for a client that reads slower than the source sends.
const PROXY_BUFFERED_CHUNKS: usize = 64;

fn upstream_error(error: &reqwest::Error) -> io::Error {
    let message = if error.is_timeout() {
        "The channel source timed out."
    } else if error.is_connect() {
        "The channel source connection failed."
    } else {
        "The channel source stopped unexpectedly."
    };
    io::Error::other(message)
}

async fn connect_channel_upstream(
    client: &reqwest::Client,
    stream_url: &str,
) -> io::Result<reqwest::Response> {
    let response = client
        .get(stream_url)
        .send()
        .await
        .map_err(|error| upstream_error(&error))?;

    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "The channel source returned HTTP {}.",
            response.status().as_u16()
        )));
    }

    Ok(response)
}

fn upstream_reconnect_delay(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(3);
    Duration::from_millis(500 * (1_u64 << exponent))
}

/// The first connection to the source. When another stream was just closed
/// for this one, a refusal is tried again for a few seconds.
async fn connect_first_upstream(
    client: &reqwest::Client,
    stream_url: &str,
    replaced: bool,
) -> io::Result<reqwest::Response> {
    let deadline = tokio::time::Instant::now() + REPLACED_UPSTREAM_CONNECT_WINDOW;
    let mut attempt = 0_u32;
    loop {
        match connect_channel_upstream(client, stream_url).await {
            Ok(response) => return Ok(response),
            Err(error) => {
                attempt += 1;
                let delay = upstream_reconnect_delay(attempt);
                if !replaced || tokio::time::Instant::now() + delay > deadline {
                    return Err(error);
                }
                tokio::time::sleep(delay).await;
            }
        }
    }
}

async fn handler_stream(
    Path((library_id, channel_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<StreamQuery>,
) -> Result<Response> {
    // Resolve the stream URL first: it checks the requester's access, and
    // taking a slot may stop another viewer's stream.
    let stream_url = mc
        .get_channel_stream_url(&library_id, &channel_id, query.quality, &user)
        .await?;

    // Enforce concurrent stream limit: the slot is freed when it is dropped.
    let (slot, replaced) = mc
        .acquire_proxy_stream_slot(&library_id, &channel_id)
        .await?;

    // Proxy the stream through our server
    let client = reqwest::Client::builder()
        .connect_timeout(UPSTREAM_CONNECT_TIMEOUT)
        .read_timeout(UPSTREAM_READ_TIMEOUT)
        .build()
        .map_err(|_| {
            Error::ChannelStreamUnavailable(
                "Unable to initialize the channel source connection.".to_string(),
            )
        })?;
    // A newer request for the slot cancels this one while it connects too.
    let cancel = slot.cancel.clone();
    let upstream = tokio::select! {
        upstream = connect_first_upstream(&client, &stream_url, replaced) => {
            upstream.map_err(|error| Error::ChannelStreamUnavailable(error.to_string()))?
        }
        _ = cancel.cancelled() => {
            return Err(Error::ChannelStreamUnavailable(
                "The stream was replaced by a newer one.".to_string(),
            ));
        }
    };

    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("video/mp2t")
        .to_string();

    // The source is read by a task of its own: it ends, closing the source
    // connection and freeing the slot, when the client goes away or a newer
    // stream needs the slot, even if the client stopped reading.
    let (tx, rx) = tokio::sync::mpsc::channel(PROXY_BUFFERED_CHUNKS);
    tokio::spawn(async move {
        let result = tokio::select! {
            result = forward_channel_stream(&client, &stream_url, upstream, &tx, &library_id, &channel_id) => result,
            _ = cancel.cancelled() => {
                log_info(
                    LogServiceType::Source,
                    format!("IPTV stream closed for a newer one: library {}, channel {}", library_id, channel_id),
                );
                Ok(())
            }
            _ = tx.closed() => Ok(()),
        };
        if let Err(error) = result {
            log_error(
                LogServiceType::Source,
                format!(
                    "IPTV stream ended for library {}, channel {}: {}",
                    library_id, channel_id, error
                ),
            );
            let _ = tx.try_send(Err(error));
        }
        // The source connection is closed by now: the slot can go.
        drop(slot);
    });
    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));

    let response = Response::builder()
        .header(CONTENT_TYPE, content_type)
        .header(TRANSFER_ENCODING, "chunked")
        .body(body)
        .map_err(|e| Error::Error(format!("Failed to build response: {}", e)))?;

    Ok(response)
}

/// Sends the source to the client, reconnecting to it when it drops. Returns
/// when the client is gone, or with an error when the source can't be restored.
async fn forward_channel_stream(
    client: &reqwest::Client,
    stream_url: &str,
    upstream: reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<io::Result<Bytes>>,
    library_id: &str,
    channel_id: &str,
) -> io::Result<()> {
    let mut next_upstream = Some(upstream);
    let mut reconnect_attempts = 0_u32;

    loop {
        let response = if let Some(response) = next_upstream.take() {
            response
        } else {
            match connect_channel_upstream(client, stream_url).await {
                Ok(response) => response,
                Err(error) => {
                    reconnect_attempts += 1;
                    if reconnect_attempts > MAX_UPSTREAM_RECONNECT_ATTEMPTS {
                        return Err(error);
                    }
                    tokio::time::sleep(upstream_reconnect_delay(reconnect_attempts)).await;
                    continue;
                }
            }
        };

        let mut received_bytes = 0_usize;
        let mut byte_stream = response.bytes_stream();
        while let Some(chunk) = byte_stream.next().await {
            match chunk {
                Ok(chunk) => {
                    received_bytes = received_bytes.saturating_add(chunk.len());
                    if received_bytes >= STABLE_UPSTREAM_BYTES {
                        reconnect_attempts = 0;
                    }
                    if tx.send(Ok(chunk)).await.is_err() {
                        return Ok(());
                    }
                }
                Err(_) => break,
            }
        }

        reconnect_attempts += 1;
        if reconnect_attempts > MAX_UPSTREAM_RECONNECT_ATTEMPTS {
            return Err(io::Error::other(
                "The channel source could not be restored after repeated attempts.",
            ));
        }

        log_info(
            LogServiceType::Source,
            format!(
                "IPTV source disconnected for library {}, channel {}; reconnecting ({}/{})",
                library_id, channel_id, reconnect_attempts, MAX_UPSTREAM_RECONNECT_ATTEMPTS
            ),
        );
        tokio::time::sleep(upstream_reconnect_delay(reconnect_attempts)).await;
    }
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
    let quality_key = query.quality.as_deref().unwrap_or("best");
    let session_key = format!("{}:{}:{}", library_id, channel_id, quality_key);
    let (_output_dir, playlist_path) = mc
        .get_or_create_hls_session(&library_id, &channel_id, query.quality.clone(), &user)
        .await?;

    // Wait for the playlist file to be created by FFmpeg
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(PLAYLIST_READY_TIMEOUT_MS);

    loop {
        if let Ok(meta) = tokio::fs::metadata(&playlist_path).await {
            if meta.len() > 0 {
                break;
            }
        }
        let session_is_running = mc.hls_sessions.read().await.contains_key(&session_key);
        if !session_is_running {
            return Err(Error::ChannelStreamUnavailable(
                "The channel source stopped before producing a playable stream.".to_string(),
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::ChannelStreamUnavailable(
                "Timed out waiting for the channel source to produce a playable stream."
                    .to_string(),
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
        .header(CACHE_CONTROL, "no-cache, no-store")
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
    let is_valid = segment.len() == 12
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
            std::io::ErrorKind::NotFound => {
                Error::NotFound(format!("Segment not found: {}", segment))
            }
            _ => Error::Error(format!("Failed to open segment: {}", e)),
        })?;

    let file_size = file.metadata().await.map(|m| m.len()).unwrap_or(0);

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut response_builder = Response::builder()
        .header(CONTENT_TYPE, "video/mp2t")
        .header(CACHE_CONTROL, "public, max-age=60");
    if file_size > 0 {
        response_builder = response_builder.header(CONTENT_LENGTH, file_size);
    }
    let response = response_builder
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_reconnect_backoff_is_bounded() {
        assert_eq!(upstream_reconnect_delay(1), Duration::from_millis(500));
        assert_eq!(upstream_reconnect_delay(2), Duration::from_secs(1));
        assert_eq!(upstream_reconnect_delay(3), Duration::from_secs(2));
        assert_eq!(upstream_reconnect_delay(4), Duration::from_secs(4));
        assert_eq!(upstream_reconnect_delay(20), Duration::from_secs(4));
    }
}
