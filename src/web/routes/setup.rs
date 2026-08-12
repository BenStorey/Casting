use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

/// GET /api/setup/status — is this company configured, and what roles are
/// available to hire? The SPA shows a first-run wizard when `configured` is
/// false (i.e. no cast hired yet, only the seed PM).
pub(crate) async fn setup_status_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    let proj = state.projection().unwrap_or_default();
    let has_cast = proj.agents.iter().any(|a| a.id != "pm");
    let roles: Vec<serde_json::Value> = crate::cast::ROLE_CATALOG
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "title": r.title,
                "scope": r.scope,
            })
        })
        .collect();
    Json(serde_json::json!({
        "configured": has_cast,
        "roles": roles,
    }))
}

#[derive(Deserialize)]
pub(crate) struct SetupIn {
    name: String,
    objective: String,
    #[serde(default)]
    cast: Vec<String>,
    #[serde(default)]
    owner_token: Option<String>,
}

/// POST /api/setup — the first-run wizard's submit. Hires the chosen cast
/// (idempotently), persists the owner token, then fires the owner's objective
/// as a message so `plan_onboard` kicks off the build.
pub(crate) async fn setup_handler(
    State(state): State<AppState>,
    Json(input): Json<SetupIn>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let cast_roles: Vec<String> = if input.cast.is_empty() {
        crate::cast::DEFAULT_CAST
            .iter()
            .map(|m| m.role_id.to_string())
            .collect()
    } else {
        input.cast.clone()
    };

    let hires = crate::setup::ensure_hires(&state, &cast_roles)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Persist the owner token + name so `cast run` picks up auth on restart.
    if let Some(dir) = &state.state_dir {
        let _ = crate::setup::persist_config(
            dir,
            &input.name,
            input.owner_token.as_deref().filter(|t| !t.is_empty()),
        );
    }

    // Fire the owner's objective so onboarding produces the build plan.
    let ev = Event::new(
        &state.project,
        Actor::Owner,
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: format!("setup-{}", uuid::Uuid::new_v4()),
        },
        serde_json::json!({ "body": input.objective }),
    );
    state
        .append(ev.clone())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "hires": hires,
        "objective": input.objective,
    })))
}
