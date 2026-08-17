use super::append_json;
use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use crate::workspace::secrets::ensure_no_secrets_in_text;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct MessageIn {
    body: String,
}

/// POST /api/message — the director sends a message to the PM. Persisted as a
/// durable `MessageSent` event; the PM loop is notified via the broadcast.
pub(crate) async fn message_handler(
    State(state): State<AppState>,
    Json(input): Json<MessageIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let body = input.body.trim().to_string();
    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "message must not be empty".into()));
    }
    // Reject if the message body embeds a raw secret value
    if let Some(ref secrets) = state.secrets {
        ensure_no_secrets_in_text(secrets, &body, "message body")
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    let ev = Event::new(
        &state.project,
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: format!("msg-{}", uuid::Uuid::new_v4()),
        },
        serde_json::json!({ "to": crate::actions::pm_actor_id(Some(&state.consultants)), "body": body }),
    );
    append_json(&state, ev)
}

/// POST /api/brief input: external advisor content. `source` marks provenance
/// (e.g. "ChatGPT advisor") so it's never confusable with the director's intent.
#[derive(Deserialize)]
pub(crate) struct BriefIn {
    source: Option<String>,
    subject: Option<String>,
    title: Option<String>,
    body: String,
    /// Optional image/diagram references (caption + path/URL).
    #[serde(default)]
    assets: Vec<crate::projection::BriefingAsset>,
}

/// POST /api/brief — the director imports EXTERNAL advisor content (text + optional
/// image/diagram refs) as an ADVISORY briefing. Explicitly advisory, NOT
/// authoritative: `source` records provenance, and it can inform context but
/// never sets rules (directives remain the only authority mechanism).
pub(crate) async fn brief_handler(
    State(state): State<AppState>,
    Json(input): Json<BriefIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let body = input.body.trim().to_string();
    if body.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "briefing body must not be empty".into(),
        ));
    }
    // Reject if briefing body embeds a raw secret value (check BEFORE action
    // construction to avoid borrow-after-move on body).
    if let Some(ref secrets) = state.secrets {
        ensure_no_secrets_in_text(secrets, &body, "briefing body")
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        if let Some(ref subject) = input.subject {
            ensure_no_secrets_in_text(secrets, subject, "briefing subject")
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        }
    }
    let action = crate::actions::PmAction::ImportBriefing {
        id: format!("brief-{}", uuid::Uuid::new_v4()),
        source: input.source.unwrap_or_else(|| "advisor".to_string()),
        subject: input.subject.unwrap_or_else(|| "general".to_string()),
        title: input
            .title
            .unwrap_or_else(|| "advisor briefing".to_string()),
        body,
        assets: input.assets,
    };
    let proj = state
        .projection()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::actions::validate(&action, "director", &proj, None)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let cause = Event::new(
        &state.project,
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "bootstrap".into(),
        },
        serde_json::json!({}),
    );
    let ev = action
        .to_events(&state.project, "director", &cause, "brief")
        .into_iter()
        .next()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "ImportBriefing produced no events".into(),
            )
        })?;
    append_json(&state, ev)
}

/// POST /api/request input: an EXTERNAL request (e.g. a GitHub issue/PR a
/// product user opened). `source` + `external_id` + `reporter` record where it
/// came from so the PM can triage it. NOT the director's own intent.
#[derive(Deserialize)]
pub(crate) struct RequestIn {
    /// e.g. "github" | "email" | "web".
    #[serde(default = "default_external")]
    source: String,
    external_id: Option<String>,
    title: String,
    #[serde(default)]
    body: String,
    /// Who raised it (e.g. GitHub username). Defaults to "external".
    #[serde(default = "default_external")]
    reporter: String,
    #[serde(default)]
    labels: Vec<String>,
    url: Option<String>,
}

fn default_external() -> String {
    "external".to_string()
}

/// POST /api/request — an EXTERNAL request (e.g. a GitHub issue/PR a user
/// opened). Recorded with provenance + deterministic triage; the PM can triage
/// it without it pretending to be the director's own intent.
pub(crate) async fn request_handler(
    State(state): State<AppState>,
    Json(input): Json<RequestIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "request title must not be empty".into(),
        ));
    }
    let body = input.body;
    // Reject if external request title/body embeds a raw secret value (check
    // BEFORE action construction to avoid borrow-after-move).
    if let Some(ref secrets) = state.secrets {
        ensure_no_secrets_in_text(secrets, &title, "request title")
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        if !body.is_empty() {
            ensure_no_secrets_in_text(secrets, &body, "request body")
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        }
    }
    let action = crate::actions::PmAction::ReceiveExternalRequest {
        id: format!("req-{}", uuid::Uuid::new_v4()),
        source: input.source,
        external_id: input.external_id,
        title,
        body,
        reporter: input.reporter,
        labels: input.labels,
        url: input.url,
    };
    let proj = state
        .projection()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::actions::validate(&action, proj.pm_id(), &proj, None)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let cause = Event::new(
        &state.project,
        Actor::System,
        EventType::ExternalRequestReceived,
        Aggregate {
            kind: "external_request".into(),
            id: "bootstrap".into(),
        },
        serde_json::json!({}),
    );
    let ev = action
        .to_events(&state.project, proj.pm_id(), &cause, "request")
        .into_iter()
        .next()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "ReceiveExternalRequest produced no events".into(),
            )
        })?;
    append_json(&state, ev)
}

/// POST /api/diagram input: Excalidraw JSON data + optional title.
#[derive(Deserialize)]
pub(crate) struct DiagramIn {
    title: String,
    data: String,
}

/// POST /api/diagram — save a diagram drawn in the app (Excalidraw).
pub(crate) async fn diagram_handler(
    State(state): State<AppState>,
    Json(input): Json<DiagramIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let title = input.title.trim().to_string();
    if input.data.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "diagram data must not be empty".into(),
        ));
    }
    let action = crate::actions::PmAction::SaveDiagram {
        id: format!("diagram-{}", uuid::Uuid::new_v4()),
        title,
        data: input.data,
    };
    let proj = state
        .projection()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::actions::validate(&action, proj.pm_id(), &proj, None)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let cause = Event::new(
        &state.project,
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::DiagramSaved,
        Aggregate {
            kind: "diagram".into(),
            id: "bootstrap".into(),
        },
        serde_json::json!({}),
    );
    let ev = action
        .to_events(&state.project, proj.pm_id(), &cause, "diagram")
        .into_iter()
        .next()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "SaveDiagram produced no events".into(),
            )
        })?;
    append_json(&state, ev)
}
