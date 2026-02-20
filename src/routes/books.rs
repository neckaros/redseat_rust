use std::{convert::Infallible, io::Cursor, time::Duration};

use axum::{
    body::Body,
    debug_handler,
    extract::Multipart,
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
    Json, Router,
};
use futures::{Stream, TryStreamExt};
use rs_plugin_common_interfaces::domain::{rs_ids::RsIds, ItemWithRelations};
use rs_plugin_common_interfaces::lookup::{RsLookupBook, RsLookupQuery};
use rs_plugin_common_interfaces::{ExternalImage, ImageType};
use serde_json::{json, Value};
use tokio_util::io::{ReaderStream, StreamReader};

use serde::Deserialize;

use crate::{
    domain::book::{Book, BookForUpdate},
    model::{books::BookQuery, medias::MediaQuery, users::ConnectedUser, ModelController},
    routes::{ImageRequestOptions, ImageUploadOptions},
    Error, Result,
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
        .route("/:id/image/search", get(handler_image_search))
        .route("/:id/image/fetch", post(handler_image_fetch))
        .route("/:id/image/refresh", get(handler_image_refresh))
        .route("/:id/image", post(handler_post_image))
        .with_state(mc)
}

async fn handler_list(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<BookQuery>,
) -> Result<Json<Vec<ItemWithRelations<Book>>>> {
    let books = mc.get_books(&library_id, query, &user).await?;
    Ok(Json(books))
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AddBookOptions {
    #[serde(default)]
    upsert_tags: bool,
    #[serde(default)]
    upsert_people: bool,
    #[serde(default)]
    upsert_serie: bool,
}

async fn handler_post(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(options): Query<AddBookOptions>,
    Json(item): Json<ItemWithRelations<Book>>,
) -> Result<Json<Value>> {
    let created = mc.add_book(&library_id, item, options.upsert_tags, options.upsert_people, options.upsert_serie, &user).await?;
    Ok(Json(json!(created)))
}

async fn handler_get(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<ItemWithRelations<Book>>> {
    let book = mc.get_book(&library_id, book_id, &user).await?;
    Ok(Json(book))
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
    let results = mc
        .exec_lookup_metadata_grouped(lookup_query, Some(library_id), &user, None)
        .await?;
    Ok(Json(json!(results)))
}

async fn handler_search_books_stream(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<RsLookupBook>,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    let lookup_query = RsLookupQuery::Book(query);
    let mut rx = mc
        .exec_lookup_metadata_stream_grouped(lookup_query, Some(library_id), &user, None)
        .await?;

    let stream = async_stream::stream! {
        while let Some((name, batch)) = rx.recv().await {
            if let Ok(data) = serde_json::to_string(&json!({ &name: batch })) {
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
    let reader_response = mc
        .book_image(
            &library_id,
            &book_id,
            query.kind.clone(),
            query.size.clone(),
            &user,
        )
        .await;

    if let Ok(reader_response) = reader_response {
        let headers = reader_response
            .hearders()
            .map_err(|_| Error::GenericRedseatError)?;
        let stream = ReaderStream::new(reader_response.stream);
        let body = Body::from_stream(stream);
        Ok((headers, body).into_response())
    } else if query.defaulting {
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
    } else if let Err(err) = reader_response {
        Err(Error::NotFound(format!(
            "Unable to find book image: {} {} {:?}",
            library_id, book_id, err
        )))
    } else {
        Err(Error::NotFound(format!(
            "Unable to find book image: {} {}",
            library_id, book_id
        )))
    }
}

async fn handler_image_search(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(_query): Query<ImageRequestOptions>,
) -> Result<Json<Value>> {
    let book = mc.get_book(&library_id, book_id, &user).await?;
    let title = book.item.name.clone();
    let ids: RsIds = book.item.into();
    let query = RsLookupBook {
        name: Some(title),
        ids: Some(ids),
    };
    let result = mc.get_book_images(query, Some(library_id), &user).await?;

    Ok(Json(json!(result)))
}

async fn handler_image_fetch(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(external_image): Json<ExternalImage>,
) -> Result<Json<Value>> {
    let request = external_image.url;
    let kind = external_image
        .kind
        .ok_or(Error::Error("Missing image type".to_string()))?;

    let mut reader = mc.request_to_reader(&library_id, request, &user).await?;

    mc.update_book_image(&library_id, &book_id, &kind, reader.stream, &user)
        .await?;

    Ok(Json(json!({"data": "ok"})))
}

async fn handler_image_refresh(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<ImageRequestOptions>,
) -> Result<Json<Value>> {
    let kind = query.kind.unwrap_or(ImageType::Poster);
    let book = mc
        .refresh_book_image(&library_id, &book_id, &kind, &user)
        .await?;
    Ok(Json(json!(book)))
}

#[debug_handler]
async fn handler_post_image(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<ImageUploadOptions>,
    mut multipart: Multipart,
) -> Result<Json<Value>> {
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
