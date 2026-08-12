use crate::pm::AppState;
use crate::provenance;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

/// GET /api/provenance/commit/:sha — the "why does this code exist?" chain for
/// a commit: commit → changeSet → task → requirement → decision → owner intent
/// (ADDENDUM §24–25).
pub(crate) async fn provenance_commit_handler(
    State(state): State<AppState>,
    Path(sha): Path<String>,
) -> Result<Json<provenance::ProvenanceChain>, StatusCode> {
    provenance::for_commit(&state.store, &state.project, &sha)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/provenance/task/:task_id — the reverse direction: what code,
/// requirement, and decision did this task produce? (ADDENDUM §25)
pub(crate) async fn provenance_task_handler(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<provenance::TaskProvenance>, StatusCode> {
    provenance::for_task(&state.store, &state.project, &task_id)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/provenance/decision/{id} — the audit for a decision: who proposed
/// it, what class/involvement, who decided it, and why (to the owner's message).
pub(crate) async fn provenance_decision_handler(
    State(state): State<AppState>,
    Path(decision_id): Path<String>,
) -> Result<Json<provenance::DecisionAudit>, StatusCode> {
    provenance::for_decision(&state.store, &state.project, &decision_id)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
