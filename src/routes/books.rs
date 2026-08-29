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
use rs_plugin_common_interfaces::domain::{rs_ids::RsIds, ItemWithRelations, Relations};
use rs_plugin_common_interfaces::lookup::{RsLookupBook, RsLookupQuery};
use rs_plugin_common_interfaces::{ElementType, ExternalImage, ImageType};

use crate::tools::log::{log_info, LogServiceType};
use serde_json::{json, Value};
use tokio_util::io::{ReaderStream, StreamReader};

use serde::Deserialize;

use crate::{
    domain::book::{Book, BookForUpdate},
    model::{books::BookQuery, medias::MediaQuery, users::ConnectedUser, ModelController},
    routes::{
        bind_downloads_to_book, ImageRequestOptions, ImageUploadOptions, RatingUpdateBody,
        SearchResultGroup, SseLookupSearchEvent, SseLookupSearchResult, SseSearchEvent,
    },
    Error, Result,
};

async fn first_person_name(
    mc: &ModelController,
    library_id: &str,
    relations: &Option<Relations>,
    user: &ConnectedUser,
) -> Option<String> {
    let person_id = relations
        .as_ref()
        .and_then(|r| r.people.as_ref())
        .and_then(|people| people.first())
        .map(|p| p.id.clone())?;
    mc.get_person(library_id, person_id, user)
        .await
        .ok()
        .flatten()
        .map(|p| p.name)
}

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
        .route("/:id/rating", get(handler_rating_get))
        .route("/:id/rating", patch(handler_rating_set))
        .route("/:id/search", get(handler_lookup))
        .route("/:id/searchstream", get(handler_lookup_stream))
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

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct BookMetadataSearchQuery {
    name: Option<String>,
    author: Option<String>,
    isbn13: Option<String>,
    page_key: Option<String>,
    source: Option<String>,
}

impl BookMetadataSearchQuery {
    fn sources(&self) -> Option<Vec<String>> {
        self.source.as_deref().map(|source| {
            source
                .split(',')
                .map(str::trim)
                .filter(|source| !source.is_empty())
                .map(str::to_string)
                .collect()
        })
    }

    fn into_lookup(self) -> Result<RsLookupBook> {
        let name = self.name.and_then(non_empty_value);
        let author = self.author.and_then(non_empty_value);
        let ids = self.isbn13.and_then(|isbn13| {
            let isbn13 = isbn13.trim();
            if isbn13.is_empty() {
                return None;
            }
            let mut ids = RsIds::default();
            ids.set("isbn13", isbn13);
            Some(ids)
        });

        if name.is_none() && author.is_none() && ids.is_none() {
            return Err(Error::InvalidParams(
                "At least one of name, author, or isbn13 is required".to_string(),
            ));
        }

        Ok(RsLookupBook {
            name,
            author,
            ids,
            page_key: self.page_key.and_then(non_empty_value),
        })
    }
}

fn non_empty_value(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

async fn handler_post(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(options): Query<AddBookOptions>,
    Json(item): Json<ItemWithRelations<Book>>,
) -> Result<Json<Value>> {
    let created = mc
        .add_book(
            &library_id,
            item,
            options.upsert_tags,
            options.upsert_people,
            options.upsert_serie,
            &user,
        )
        .await?;
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
    Query(query): Query<BookMetadataSearchQuery>,
) -> Result<Json<Value>> {
    let sources = query.sources();
    let lookup_query = RsLookupQuery::Book(query.into_lookup()?);
    let groups = mc
        .exec_lookup_metadata_grouped(
            lookup_query,
            Some(library_id),
            &user,
            None,
            sources.as_deref(),
        )
        .await?;
    let body: Vec<SearchResultGroup> = groups
        .into_iter()
        .map(|(source_id, source_name, data)| SearchResultGroup {
            source_id,
            source_name,
            data,
        })
        .collect();
    Ok(Json(json!(body)))
}

async fn handler_search_books_stream(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<BookMetadataSearchQuery>,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    let sources = query.sources();
    let lookup_query = RsLookupQuery::Book(query.into_lookup()?);
    let mut rx = mc
        .exec_lookup_metadata_stream_grouped(
            lookup_query,
            Some(library_id),
            &user,
            None,
            sources.as_deref(),
        )
        .await?;

    let stream = async_stream::stream! {
        while let Some((source_id, source_name, batch)) = rx.recv().await {
            if let Ok(data) = serde_json::to_string(&SseSearchEvent { source_id: &source_id, source_name: &source_name, data: &batch }) {
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

async fn handler_rating_get(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let rating = mc
        .get_media_rating(&library_id, ElementType::Book, book_id, &user)
        .await?;
    Ok(Json(json!(rating)))
}

async fn handler_rating_set(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Json(body): Json<RatingUpdateBody>,
) -> Result<Json<Value>> {
    let rating = mc
        .set_media_rating(&library_id, ElementType::Book, book_id, body.rating, &user)
        .await?;
    Ok(Json(json!(rating)))
}

async fn handler_lookup(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Json<Value>> {
    let book = mc.get_book(&library_id, book_id, &user).await?;
    let name = book.item.name.clone();
    let author = first_person_name(&mc, &library_id, &book.relations, &user).await;
    let ids: RsIds = book.item.into();
    let query = RsLookupQuery::Book(RsLookupBook {
        name: Some(name),
        author,
        ids: Some(ids),
        page_key: None,
    });
    log_info(
        LogServiceType::Source,
        format!("Executing lookup with query: {:?}", query),
    );
    let results = mc.exec_lookup(query, Some(library_id), &user, None).await?;
    Ok(Json(json!(results)))
}

async fn handler_lookup_stream(
    Path((library_id, book_id)): Path<(String, String)>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    let book = mc.get_book(&library_id, book_id.clone(), &user).await?;
    let name = book.item.name.clone();
    let author = first_person_name(&mc, &library_id, &book.relations, &user).await;
    let ids: RsIds = book.item.into();
    let query = RsLookupQuery::Book(RsLookupBook {
        name: Some(name),
        author,
        ids: Some(ids),
        page_key: None,
    });
    let mut rx = mc
        .exec_lookup_stream_grouped(query, Some(library_id), &user, None, None)
        .await?;

    let stream = async_stream::stream! {
        while let Some((source_id, source_name, mut groups)) = rx.recv().await {
            bind_downloads_to_book(&mut groups, &book_id);
            let results = SseLookupSearchResult::from_groups(&groups);
            if let Ok(data) = serde_json::to_string(&SseLookupSearchEvent {
                source_id: &source_id,
                source_name: &source_name,
                results: &results,
                downloads: &groups,
            }) {
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
        author: None,
        ids: Some(ids),
        page_key: None,
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

#[cfg(test)]
mod tests {
    use super::BookMetadataSearchQuery;
    use crate::Error;
    use axum::http::StatusCode;

    #[test]
    fn book_metadata_search_combines_title_author_and_isbn() {
        let lookup = BookMetadataSearchQuery {
            name: Some("  The Book  ".to_string()),
            author: Some("  Jane Doe  ".to_string()),
            isbn13: Some(" 9781402894626 ".to_string()),
            page_key: None,
            source: None,
        }
        .into_lookup()
        .unwrap();

        assert_eq!(lookup.name.as_deref(), Some("The Book"));
        assert_eq!(lookup.author.as_deref(), Some("Jane Doe"));
        assert_eq!(
            lookup.ids.as_ref().and_then(|ids| ids.isbn13()),
            Some("9781402894626")
        );
    }

    #[test]
    fn book_metadata_search_rejects_empty_criteria() {
        let result = BookMetadataSearchQuery {
            name: Some("  ".to_string()),
            author: None,
            isbn13: Some(" ".to_string()),
            page_key: None,
            source: Some("openlibrary".to_string()),
        }
        .into_lookup();

        let error = result.unwrap_err();
        assert!(matches!(&error, Error::InvalidParams(_)));
        assert_eq!(error.client_status_and_error().0, StatusCode::BAD_REQUEST);
    }
}
