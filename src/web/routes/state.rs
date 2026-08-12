use crate::event::Event;
use crate::pm::AppState;
use crate::projection::Projection;
use crate::store::EventStore;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::Json;
use futures::stream::{Stream, StreamExt};
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Duration;

/// GET /api/state — the current projection (agents, tasks, decisions, ...).
pub(crate) async fn state_handler(
    State(state): State<AppState>,
) -> Result<Json<Projection>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(proj))
}

/// GET /api/events?after=N — raw event history slice (activity stream / catch-up).
pub(crate) async fn events_handler(
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<Vec<Event>>, StatusCode> {
    let after = q.after.unwrap_or(0);
    let events = state
        .store
        .read_since(&state.project, after)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(events))
}

#[derive(Deserialize)]
pub(crate) struct EventsQuery {
    after: Option<i64>,
}

/// GET /api/events/stream — a live Server-Sent-Events feed.
///
/// Pushes every newly-appended event to connected browsers so the
/// board/chat/activity update live (§35).
///
/// Supports catch-up via `?after=N`: missed events since sequence N are
/// replayed from the store *before* the stream switches to live broadcast,
/// so a reconnecting client never loses events that happened while it was
/// offline. There is a benign race — an event appended between the `read_since`
/// and the `subscribe` may arrive via both catch-up and broadcast. The UI
/// refetches `/api/state` on every event, so duplicates are harmless
/// (idempotent projection rebuild).
pub(crate) async fn events_stream(
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let after = q.after.unwrap_or(0);
    let rx = state.subscribe();

    let catchup = state
        .store
        .read_since(&state.project, after)
        .unwrap_or_default();

    // Catch-up events first (one-shot), then live broadcasts.
    let catchup_stream = futures::stream::iter(catchup.into_iter().map(|ev| {
        let json = serde_json::to_string(&ev).unwrap_or_default();
        Ok::<_, Infallible>(SseEvent::default().event("event").data(json))
    }));

    let live = futures::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Ok(ev) => {
                let json = serde_json::to_string(&ev).unwrap_or_default();
                Some((
                    Ok::<_, Infallible>(SseEvent::default().event("event").data(json)),
                    rx,
                ))
            }
            Err(_) => None, // sender dropped — stream ends
        }
    });

    Sse::new(catchup_stream.chain(live))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
