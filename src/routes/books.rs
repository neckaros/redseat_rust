use axum::{
    extract::{Path, Query, State},
    routing::{delete, get, patch, post},
    Json, Router,
};
use serde_json::{json, Value};

use crate::{
    domain::book::{Book, BookForUpdate},
    model::{books::BookQuery, medias::MediaQuery, users::ConnectedUser, ModelController},
    Result,
};

pub fn routes(mc: ModelController) -> Router {
    Router::new()
        .route("/", get(handler_list))
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
