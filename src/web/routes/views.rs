use crate::pm::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

/// GET /api/context/{actor} — the assembled operating context for an actor
/// (agent id, "owner", or "pm"): objective, priorities, their tasks, the
/// governance directives that apply to them, risks, and open decisions.
pub(crate) async fn context_handler(
    State(state): State<AppState>,
    Path(actor): Path<String>,
) -> Result<Json<crate::context::AgentContext>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(proj.context_for(&actor)))
}

/// GET /api/persona/{agent_id} — the derived persona/CV card for a hired agent.
pub(crate) async fn persona_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<crate::persona::Persona>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match proj.persona_for(&agent_id) {
        Some(p) => Ok(Json(p)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// GET /api/model — the operating picture: what the models are currently
/// seeing (objective, priorities, governance, knowledge, per-actor contexts,
/// and any mechanical drift signals). The owner's "why is it prioritizing that
/// way?" / "what does it believe?" debug surface. Pure derivation.
pub(crate) async fn model_handler(
    State(state): State<AppState>,
) -> Result<Json<crate::mental::OperatingModel>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(proj.operating_model()))
}
