use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{
    domain::{
        backup::{BackupFileProgress, BackupMessage},
        episode::EpisodesMessage,
        library::{LibraryMessage, LibraryRole, LibraryStatusMessage},
        media::{ConvertMessage, MediasMessage, UploadProgressMessage},
        movie::MoviesMessage,
        people::PeopleMessage,
        serie::SeriesMessage,
        tag::TagMessage,
    },
    model::{media_progresses::MediasProgressMessage, media_ratings::MediasRatingMessage, users::ConnectedUser, ModelController},
};

/// Unified SSE event that wraps all possible event types
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SseEvent {
    Library(LibraryMessage),
    LibraryStatus(LibraryStatusMessage),
    Medias(MediasMessage),
    UploadProgress(UploadProgressMessage),
    ConvertProgress(ConvertMessage),
    Episodes(EpisodesMessage),
    Series(SeriesMessage),
    Movies(MoviesMessage),
    People(PeopleMessage),
    Tags(TagMessage),
    Backups(BackupMessage),
    BackupsFiles(BackupFileProgress),
    MediaProgress(MediasProgressMessage),
    MediaRating(MediasRatingMessage),
}

impl SseEvent {
    /// Returns the event name for SSE "event:" field
    pub fn event_name(&self) -> &'static str {
        match self {
            SseEvent::Library(_) => "library",
            SseEvent::LibraryStatus(_) => "library-status",
            SseEvent::Medias(_) => "medias",
            SseEvent::UploadProgress(_) => "upload_progress",
            SseEvent::ConvertProgress(_) => "convert_progress",
            SseEvent::Episodes(_) => "episodes",
            SseEvent::Series(_) => "series",
            SseEvent::Movies(_) => "movies",
            SseEvent::People(_) => "people",
            SseEvent::Tags(_) => "tags",
            SseEvent::Backups(_) => "backups",
            SseEvent::BackupsFiles(_) => "backups-files",
            SseEvent::MediaProgress(_) => "media_progress",
            SseEvent::MediaRating(_) => "media_rating",
        }
    }

    /// Returns the library ID if the event is library-scoped
    pub fn library_id(&self) -> Option<&str> {
        match self {
            SseEvent::Library(m) => Some(&m.library.id),
            SseEvent::LibraryStatus(m) => Some(&m.library),
            SseEvent::Medias(m) => Some(&m.library),
            SseEvent::UploadProgress(m) => Some(&m.library),
            SseEvent::ConvertProgress(m) => Some(&m.library),
            SseEvent::Episodes(m) => Some(&m.library),
            SseEvent::Series(m) => Some(&m.library),
            SseEvent::Movies(m) => Some(&m.library),
            SseEvent::People(m) => Some(&m.library),
            SseEvent::Tags(m) => Some(&m.library),
            SseEvent::Backups(m) => m.backup.backup.library.as_deref(),
            SseEvent::BackupsFiles(m) => m.library.as_deref(),
            SseEvent::MediaProgress(m) => Some(&m.library),
            SseEvent::MediaRating(m) => Some(&m.library),
        }
    }

    /// Checks if this event should be sent to the given user
    pub fn should_send_to(&self, user: &ConnectedUser) -> bool {
        use crate::model::users::UserRole;

        match self {
            // Admin-only events
            SseEvent::LibraryStatus(m) => {
                user.check_library_role(&m.library, LibraryRole::Admin)
                    .is_ok()
            }
            SseEvent::BackupsFiles(_) => user.check_role(&UserRole::Admin).is_ok(),

            // Backup events: library admin or server admin
            SseEvent::Backups(m) => {
                if let Some(library) = &m.backup.backup.library {
                    user.check_library_role(library, LibraryRole::Admin).is_ok()
                } else {
                    user.check_role(&UserRole::Admin).is_ok()
                }
            }

            // User-specific events: only send to the user whose progress this is
            SseEvent::MediaProgress(m) => {
                user.user_id()
                    .map(|uid| uid == m.progress.user_ref)
                    .unwrap_or(false)
            }

            // User-specific events: only send to the user whose rating this is
            SseEvent::MediaRating(m) => {
                user.user_id()
                    .map(|uid| uid == m.rating.user_ref)
                    .unwrap_or(false)
            }

            // Library-scoped events (read access required)
            _ => {
                if let Some(lib_id) = self.library_id() {
                    user.check_library_role(lib_id, LibraryRole::Read).is_ok()
                } else {
                    false
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SseQueryParams {
    /// Optional: filter to specific libraries (comma-separated)
    pub libraries: Option<String>,
}

pub fn routes(mc: ModelController) -> Router {
    Router::new()
        .route("/", get(handler_sse))
        .with_state(mc)
}

async fn handler_sse(
    State(mc): State<ModelController>,
    user: ConnectedUser,
    Query(params): Query<SseQueryParams>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    // Parse library filter if provided
    let library_filter: Option<Vec<String>> = params
        .libraries
        .map(|s| s.split(',').map(|l| l.trim().to_string()).collect());

    // Subscribe to broadcast channel
    let mut rx = mc.sse_tx.subscribe();

    // Create stream that filters events for this user
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    // Check if event should be sent to this user
                    if !event.should_send_to(&user) {
                        continue;
                    }

                    // Apply library filter if specified
                    if let Some(ref filter) = library_filter {
                        if let Some(lib_id) = event.library_id() {
                            if !filter.contains(&lib_id.to_string()) {
                                continue;
                            }
                        }
                    }

                    // Serialize and send event
                    if let Ok(data) = serde_json::to_string(&event) {
                        yield Ok(Event::default()
                            .event(event.event_name())
                            .data(data));
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Client fell behind, skip missed events
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    // Channel closed, end stream
                    break;
                }
            }
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    )
}
