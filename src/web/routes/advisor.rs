use super::append_json;
use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

/// POST /api/advisor/message input: an owner→advisor message. Appends to the
/// private advisor thread, ISOLATED from the PM's context until a handoff.
#[derive(Deserialize)]
pub(crate) struct AdvisorMsgIn {
    body: String,
}

/// POST /api/advisor/message — an owner→advisor message. Appends to the PRIVATE
/// advisor thread, which is isolated from the PM's context until a handoff.
pub(crate) async fn advisor_message_handler(
    State(state): State<AppState>,
    Json(input): Json<AdvisorMsgIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let body = input.body.trim().to_string();
    if body.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "advisor message must not be empty".into(),
        ));
    }
    let ev = Event::new(
        &state.project,
        Actor::Owner,
        EventType::AdvisorMessageSent,
        Aggregate {
            kind: "advisor_thread".into(),
            id: format!("am-{}", uuid::Uuid::new_v4()),
        },
        serde_json::json!({ "to": "advisor", "body": body }),
    );
    append_json(&state, ev)
}

/// POST /api/advisor/handoff input: turn the advisor thread into a Briefing
/// the PM reads. `summary` is the (owner/LLM) distilled take; we record it as
/// an AdvisoryBriefing provenanced "advisor".
#[derive(Deserialize)]
pub(crate) struct AdvisorHandoffIn {
    title: Option<String>,
    subject: Option<String>,
    summary: String,
}

/// POST /api/advisor/handoff — turn the owner↔advisor strategic conversation into
/// an AdvisoryBriefing the PM DOES read (source "advisor"). This is the explicit
/// integration point between the owner's two direct roles (PM + advisor).
pub(crate) async fn advisor_handoff_handler(
    State(state): State<AppState>,
    Json(input): Json<AdvisorHandoffIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let summary = input.summary.trim().to_string();
    if summary.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "handoff summary must not be empty".into(),
        ));
    }
    let subject = input.subject.unwrap_or_default().trim().to_string();
    let ev = Event::new(
        &state.project,
        Actor::Owner,
        EventType::AdvisorHandoff,
        Aggregate {
            kind: "briefing".into(),
            id: format!("brief-{}", uuid::Uuid::new_v4()),
        },
        serde_json::json!({
            "source": "advisor",
            "subject": subject,
            "title": input.title.unwrap_or_else(|| "Advisor handoff".into()),
            "body": summary,
        }),
    );
    append_json(&state, ev)
}
