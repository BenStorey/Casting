use crate::pm::AppState;
use crate::projection::DecisionStatus;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

/// An item sitting in the owner's inbox (a decision awaiting a verdict).
#[derive(Serialize)]
pub(crate) struct InboxItem {
    id: String,
    subject: String,
    recommendation: Option<String>,
    options: serde_json::Value,
    class: String,
    involvement: String,
}

#[derive(Serialize)]
pub(crate) struct Inbox {
    items: Vec<InboxItem>,
    unread: usize,
}

/// GET /api/inbox — what the owner needs to decide on right now.
pub(crate) async fn inbox_handler(
    State(state): State<AppState>,
) -> Result<Json<Inbox>, StatusCode> {
    let proj = state
        .projection()
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
            class: format!("{:?}", d.class).to_lowercase(),
            involvement: format!("{:?}", d.involvement).to_lowercase(),
        })
        .collect();
    let unread = items.len();
    Ok(Json(Inbox { items, unread }))
}
