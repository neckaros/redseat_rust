use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::{delete, get, patch, post},
    Json, Router,
};
use futures::Stream;
use rs_plugin_common_interfaces::lookup::{RsLookupBook, RsLookupQuery};
use serde_json::{json, Value};

use crate::{
    domain::book::{Book, BookForUpdate},
    model::{books::BookQuery, medias::MediaQuery, users::ConnectedUser, ModelController},
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
