//! Axum web server for the vertical slice: JSON API over the projections,
//! owner inbox endpoints, SSE realtime, and the embedded React SPA.
//!
//! Serves everything from ONE binary (brief §26/§29/§31): the API and the
//! compiled frontend are both handled here, so `cast run` stays a single
//! self-contained native executable whose only output is a local workspace.

use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use crate::projection::{DecisionStatus, Projection};
use crate::provenance;
use crate::store::EventStore;
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures::stream::{Stream, StreamExt};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::Duration;

/// Embedded build output of the React SPA (see `frontend/`). `cast run` ships
/// this inside the binary, so end users never build or host the frontend.
#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct Assets;

/// An item sitting in the owner's inbox (a decision awaiting a verdict).
#[derive(Serialize)]
struct InboxItem {
    id: String,
    subject: String,
    recommendation: Option<String>,
    options: serde_json::Value,
}

#[derive(Serialize)]
struct Inbox {
    items: Vec<InboxItem>,
    unread: usize,
}

#[derive(Deserialize)]
struct MessageIn {
    body: String,
}

#[derive(Deserialize)]
struct DecisionIn {
    decision_id: String,
    subject: String,
    approved: bool,
    #[serde(default)]
    note: Option<String>,
}

/// Build the full router for a project's runtime state.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/state", get(state_handler))
        .route("/api/events", get(events_handler))
        .route("/api/events/stream", get(events_stream))
        .route("/api/inbox", get(inbox_handler))
        .route("/api/message", axum::routing::post(message_handler))
        .route("/api/decision", axum::routing::post(decision_handler))
        .route("/api/provenance/commit/:sha", get(provenance_commit_handler))
        .route("/api/provenance/task/:task_id", get(provenance_task_handler))
        // The embedded SPA (and SPA route fallback) handles everything else.
        .fallback(static_handler)
        .with_state(state)
}

/// GET /api/state — the current projection (agents, tasks, decisions, ...).
async fn state_handler(State(state): State<AppState>) -> Result<Json<Projection>, StatusCode> {
    let proj = Projection::build(&state.store, &state.project)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(proj))
}

/// GET /api/events?after=N — raw event history slice (activity stream / catch-up).
async fn events_handler(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<EventsQuery>,
) -> Result<Json<Vec<Event>>, StatusCode> {
    let after = q.after.unwrap_or(0);
    let events = state
        .store
        .read_since(&state.project, after)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(events))
}

#[derive(Deserialize)]
struct EventsQuery {
    after: Option<i64>,
}

/// GET /api/inbox — what the owner needs to decide on right now.
async fn inbox_handler(State(state): State<AppState>) -> Result<Json<Inbox>, StatusCode> {
    let proj = Projection::build(&state.store, &state.project)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let items: Vec<InboxItem> = proj
        .decisions
        .iter()
        .filter(|d| d.status == DecisionStatus::Proposed)
        .map(|d| InboxItem {
            id: d.id.clone(),
            subject: d.subject.clone(),
            recommendation: d.recommendation.clone(),
            options: d.options.clone(),
        })
        .collect();
    let unread = items.len();
    Ok(Json(Inbox { items, unread }))
}

/// POST /api/message — the owner sends a message to the PM. Persisted as a
/// durable `MessageSent` event; the PM loop is notified via the broadcast.
async fn message_handler(
    State(state): State<AppState>,
    Json(input): Json<MessageIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let body = input.body.trim().to_string();
    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "message must not be empty".into()));
    }
    let ev = Event::new(
        &state.project,
        Actor::Owner,
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: format!("msg-{}", uuid::Uuid::new_v4()),
        },
        serde_json::json!({ "to": "pm", "body": body }),
    );
    let stored = state
        .append(ev)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(stored))
}

/// POST /api/decision — the owner records a verdict on a proposed decision.
/// Durable `OwnerDecisionRecorded`; the PM loop reacts and drives follow-up.
async fn decision_handler(
    State(state): State<AppState>,
    Json(input): Json<DecisionIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let ev = Event::new(
        &state.project,
        Actor::Owner,
        EventType::OwnerDecisionRecorded,
        Aggregate {
            kind: "decision".into(),
            id: input.decision_id.clone(),
        },
        serde_json::json!({
            "subject": input.subject,
            "approved": input.approved,
            "note": input.note.unwrap_or_default(),
        }),
    );
    let stored = state
        .append(ev)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(stored))
}

/// GET /api/events/stream — Server-Sent Events: pushes each newly-appended
/// event to connected browsers so the board/chat/activity update live (§35).
///
/// Supports catch-up via `?after=N`: missed events since sequence N are
/// replayed from the store *before* the stream switches to live broadcast,
/// so a reconnecting client never loses events that happened while it was
/// offline. There is a benign race — an event appended between the `read_since`
/// and the `subscribe` may arrive via both catch-up and broadcast. The UI
/// refetches `/api/state` on every event, so duplicates are harmless
/// (idempotent projection rebuild).
async fn events_stream(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<EventsQuery>,
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

/// GET /api/provenance/commit/:sha — the "why does this code exist?" chain for
/// a commit: commit → changeSet → task → requirement → decision → owner intent
/// (ADDENDUM §24–25).
async fn provenance_commit_handler(
    State(state): State<AppState>,
    axum::extract::Path(sha): axum::extract::Path<String>,
) -> Result<Json<provenance::ProvenanceChain>, StatusCode> {
    provenance::for_commit(&state.store, &state.project, &sha)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/provenance/task/:task_id — the reverse direction: what code,
/// requirement, and decision did this task produce? (ADDENDUM §25)
async fn provenance_task_handler(
    State(state): State<AppState>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Result<Json<provenance::TaskProvenance>, StatusCode> {
    provenance::for_task(&state.store, &state.project, &task_id)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Serve the embedded SPA. Real files serve directly; unknown paths fall back
/// to index.html so client-side routing works. Unknown `/api/*` paths return a
/// JSON 404 instead of falling through to the SPA (so API clients get a proper
/// error, not an HTML page).
async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // Unknown API routes get a JSON 404, never the SPA fallback.
    if path.starts_with("api/") {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"error\":\"not found\"}",
        )
            .into_response();
    }

    match Assets::get(path) {
        Some(file) => {
            let mime = mime_for(path);
            ([(header::CONTENT_TYPE, mime)], file.data).into_response()
        }
        None => {
            // SPA route fallback: a bare path with no extension -> index.html.
            if !path.contains('.') {
                if let Some(index) = Assets::get("index.html") {
                    return ([(header::CONTENT_TYPE, mime_for("index.html"))], index.data)
                        .into_response();
                }
            }
            (StatusCode::NOT_FOUND, "not found").into_response()
        }
    }
}

fn mime_for(path: &str) -> HeaderValue {
    let mime = match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "webmanifest" => "application/manifest+json",
        "map" => "application/json",
        _ => "application/octet-stream",
    };
    HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream"))
}
