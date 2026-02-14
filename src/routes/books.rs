use std::{convert::Infallible, io::Cursor, time::Duration};

use axum::{
    body::Body,
    debug_handler,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    response::sse::{Event, KeepAlive, Sse},
    extract::Multipart,
    routing::{delete, get, patch, post},
    Json, Router,
};
use futures::{Stream, TryStreamExt};
use rs_plugin_common_interfaces::lookup::{RsLookupBook, RsLookupQuery};
use rs_plugin_common_interfaces::ImageType;
use serde_json::{json, Value};
use tokio_util::io::{ReaderStream, StreamReader};

use crate::{
    domain::book::{Book, BookForUpdate},
    model::{books::BookQuery, medias::MediaQuery, users::ConnectedUser, ModelController},
    routes::{ImageRequestOptions, ImageUploadOptions},
    Error,
    Result,
};

pub fn routes(mc: ModelController) -> Router {
    Router::new()
        .route("/", get(handler_list))
        .route("/search", get(handler_search_books))
        .route("/searchstream", get(handler_search_books_stream))
        .route("/", post(handler_post))
        .route("/:id", get(handler_get))
        .route("/:id", patch(handler_patch))
        .route("/:id", delete(handler_delete))
        .route("/:id/medias", get(handler_medias))
        .route("/:id/image", get(handler_image))
        .route("/:id/image", post(handler_post_image))
        .with_state(mc)
}

async fn handler_list(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<BookQuery>,
) -> Result<Json<Value>> {
    let books = mc.get_books(&library_id, query, &user).await?;
    Ok(Json(json!(books)))
}

async fn handler_post(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(book): Json<Book>,
) -> Result<Json<Value>> {
    let created = mc.add_book(&library_id, book, &user).await?;
    Ok(Json(json!(created)))
}

async fn handler_get(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let book = mc.get_book(&library_id, book_id, &user).await?;
    Ok(Json(json!(book)))
}

async fn handler_patch(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(update): Json<BookForUpdate>,
) -> Result<Json<Value>> {
    let updated = mc.update_book(&library_id, book_id, update, &user).await?;
    Ok(Json(json!(updated)))
}

async fn handler_delete(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let deleted = mc.remove_book(&library_id, &book_id, &user).await?;
    Ok(Json(json!(deleted)))
}

async fn handler_search_books(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<RsLookupBook>,
) -> Result<Json<Value>> {
    let lookup_query = RsLookupQuery::Book(query);
    let results = mc.exec_lookup_metadata(lookup_query, Some(library_id), &user, None).await?;
    Ok(Json(json!(results)))
}

async fn handler_search_books_stream(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<RsLookupBook>,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    let lookup_query = RsLookupQuery::Book(query);
    let mut rx = mc.exec_lookup_metadata_stream(lookup_query, Some(library_id), &user, None).await?;

    let stream = async_stream::stream! {
        while let Some(batch) = rx.recv().await {
            if let Ok(data) = serde_json::to_string(&batch) {
                yield Ok(Event::default().event("results").data(data));
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    ))
}

async fn handler_medias(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let medias = mc
        .get_medias(
            &library_id,
            MediaQuery {
                book: Some(book_id),
                ..Default::default()
            },
            &user,
        )
        .await?;
    Ok(Json(json!(medias)))
}

async fn handler_image(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<ImageRequestOptions>,
) -> Result<Response> {
    if query.kind.as_ref().is_some_and(|kind| kind != &ImageType::Poster) {
        return Err(Error::NotFound("Only poster image type is supported for books".to_string()));
    }

    let reader_response = mc
        .book_image(
            &library_id,
            &book_id,
            Some(ImageType::Poster),
            query.size,
            &user,
        )
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
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<ImageUploadOptions>,
    mut multipart: Multipart,
) -> Result<Json<Value>> {
    if query.kind != ImageType::Poster {
        return Err(Error::NotFound("Only poster image type is supported for books".to_string()));
    }

    while let Some(field) = multipart.next_field().await.unwrap() {
        let mut reader = StreamReader::new(field.map_err(|multipart_error| {
            std::io::Error::new(std::io::ErrorKind::Other, multipart_error)
        }));

        let mut data = Vec::new();
        tokio::io::copy(&mut reader, &mut data).await?;
        let reader = Box::pin(Cursor::new(data));

        mc.update_book_image(&library_id, &book_id, &query.kind, reader, &user)
            .await?;
    }

    Ok(Json(json!({"data": "ok"})))
}
