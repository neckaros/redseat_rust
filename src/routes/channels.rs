use axum::{
    body::Body,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use http::header::{CONTENT_TYPE, TRANSFER_ENCODING};
use serde_json::{json, Value};
use tokio_util::io::ReaderStream;

use crate::{
    domain::channel::Channel,
    model::{
        channels::{ChannelQuery, ImportRequest, StreamQuery},
        users::ConnectedUser,
        ModelController,
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
        .route("/:id/stream", get(handler_stream))
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

async fn handler_stream(
    Path((library_id, channel_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<StreamQuery>,
) -> Result<Response> {
    let stream_url = mc
        .get_channel_stream_url(&library_id, &channel_id, query.quality, &user)
        .await?;

    // Proxy the stream through our server
    let client = reqwest::Client::new();
    let upstream = client
        .get(&stream_url)
        .send()
        .await
        .map_err(|e| Error::Error(format!("Failed to connect to stream: {}", e)))?;

    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("video/mp2t")
        .to_string();

    let byte_stream = upstream.bytes_stream();
    let body = Body::from_stream(byte_stream);

    let response = Response::builder()
        .header(CONTENT_TYPE, content_type)
        .header(TRANSFER_ENCODING, "chunked")
        .body(body)
        .map_err(|e| Error::Error(format!("Failed to build response: {}", e)))?;

    Ok(response)
}
