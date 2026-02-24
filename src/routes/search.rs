use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Json, Router,
};
use futures::Stream;
use rs_plugin_common_interfaces::lookup::{RsLookupBook, RsLookupMovie, RsLookupQuery};
use serde_json::{json, Value};

use crate::{
    model::{users::ConnectedUser, ModelController},
    routes::{SearchQuery, SearchResultGroup, SseSearchEvent},
    Result,
};

pub fn routes(mc: ModelController) -> Router {
    Router::new()
        .route("/search", get(handler_search_global))
        .route("/searchstream", get(handler_search_global_stream))
        .with_state(mc)
}

async fn handler_search_global(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<SearchQuery<RsLookupBook>>,
) -> Result<Json<Value>> {
    let sources = query.sources();

    let movie_lookup = RsLookupMovie {
        name: query.lookup.name.clone(),
        ids: query.lookup.ids.clone(),
        page_key: query.lookup.page_key.clone(),
    };
    let serie_lookup = RsLookupMovie {
        name: query.lookup.name.clone(),
        ids: query.lookup.ids.clone(),
        page_key: query.lookup.page_key.clone(),
    };

    let (movie_results, serie_results, book_results) = tokio::try_join!(
        mc.search_movie(&library_id, movie_lookup, sources.clone(), &user),
        mc.search_serie(&library_id, serie_lookup, sources.clone(), &user),
        mc.exec_lookup_metadata_grouped(
            RsLookupQuery::Book(query.lookup),
            Some(library_id.clone()),
            &user,
            None,
            sources.as_deref(),
        ),
    )?;

    let mut groups = movie_results;
    groups.extend(serie_results);
    groups.extend(book_results);

    let body: Vec<SearchResultGroup> = groups
        .into_iter()
        .map(|(source_id, source_name, data)| SearchResultGroup { source_id, source_name, data })
        .collect();

    Ok(Json(json!(body)))
}

async fn handler_search_global_stream(
    Path(library_id): Path<String>,
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(query): Query<SearchQuery<RsLookupBook>>,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    let sources = query.sources();

    let movie_lookup = RsLookupMovie {
        name: query.lookup.name.clone(),
        ids: query.lookup.ids.clone(),
        page_key: query.lookup.page_key.clone(),
    };
    let serie_lookup = RsLookupMovie {
        name: query.lookup.name.clone(),
        ids: query.lookup.ids.clone(),
        page_key: query.lookup.page_key.clone(),
    };

    let mut movie_rx = mc
        .search_movie_stream(&library_id, movie_lookup, sources.clone(), &user)
        .await?;
    let mut serie_rx = mc
        .search_serie_stream(&library_id, serie_lookup, sources.clone(), &user)
        .await?;
    let mut book_rx = mc
        .exec_lookup_metadata_stream_grouped(
            RsLookupQuery::Book(query.lookup),
            Some(library_id.clone()),
            &user,
            None,
            sources.as_deref(),
        )
        .await?;

    let (tx, rx) = tokio::sync::mpsc::channel(32);

    let tx1 = tx.clone();
    tokio::spawn(async move {
        while let Some(item) = movie_rx.recv().await {
            if tx1.send(item).await.is_err() {
                break;
            }
        }
    });

    let tx2 = tx.clone();
    tokio::spawn(async move {
        while let Some(item) = serie_rx.recv().await {
            if tx2.send(item).await.is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        while let Some(item) = book_rx.recv().await {
            if tx.send(item).await.is_err() {
                break;
            }
        }
    });

    let stream = async_stream::stream! {
        let mut rx = rx;
        while let Some((source_id, source_name, batch)) = rx.recv().await {
            if let Ok(data) = serde_json::to_string(&SseSearchEvent {
                source_id: &source_id,
                source_name: &source_name,
                data: &batch,
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
