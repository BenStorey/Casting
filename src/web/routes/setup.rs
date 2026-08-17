use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;

/// GET /api/setup/status — is this company configured, and what roles are
/// available to hire? The SPA shows a first-run wizard when `configured` is
/// false (i.e. no cast hired yet, only the seed PM).
pub(crate) async fn setup_status_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    let proj = match state.projection() {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to get projection: {e}");
            Default::default()
        }
    };
    let has_cast = proj.agents.iter().any(|a| !proj.is_pm(&a.id));
    let roles: Vec<serde_json::Value> = state
        .consultants
        .known_roles()
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
    /// What the director wants to be called (the PM will use this).
    #[serde(default)]
    director_name: Option<String>,
    /// Experience level calibration: "novice" | "somewhat" | "confident".
    #[serde(default)]
    experience_level: Option<String>,
    /// LLM provider API key. Stored for D2 wiring.
    #[serde(default)]
    api_key: Option<String>,
    /// Which LLM provider the key belongs to: "openrouter" | "openai" | "anthropic".
    #[serde(default)]
    provider: Option<String>,
    /// Optional model id for the provider (defaults when unset).
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    cast: Vec<String>,
    #[serde(default)]
    director_token: Option<String>,
}

/// POST /api/setup — the first-run wizard's submit. Hires the chosen cast
/// (idempotently), persists the director token, then fires the director's objective
/// as a message so `plan_onboard` kicks off the build.
///
/// FAIL-CLOSED against silent token rotation: persist_config would otherwise
/// overwrite an already-persisted director token. We refuse to replace a token
/// that is already persisted with a different one unless the request presents
/// the CURRENT token (the one already on disk). First-run (no previously
/// persisted token) may still SET a token.
pub(crate) async fn setup_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<SetupIn>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // `ensure_hires` handles empty (hire the whole roster from the directory);
    // a non-empty `cast` selects specific roles to hire.
    let cast_roles = input.cast;
    let hires = crate::workspace::setup::ensure_hires(&state, &cast_roles)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Persist the director token + name so `cast run` picks up auth on restart.
    if let Some(dir) = &state.state_dir {
        let director_token = input.director_token.as_deref().filter(|t| !t.is_empty());
        // Fail-closed against silent token rotation: never replace an
        // already-persisted director token with a *different* one unless the
        // request presents the current (persisted) token. First-run SET is
        // still allowed (no prior token). Requires that a state dir has been
        // attached (as `cast run` does), so this mirrors exactly what will be
        // read back — there is no other persistence seam.
        if let Some(existing) = crate::workspace::setup::read_config(dir)
            .and_then(|cfg| cfg.director_token)
            .filter(|t| !t.is_empty())
        {
            let replacing = director_token.is_none_or(|incoming| incoming != existing);
            if replacing && !crate::workspace::auth::authorized(&headers, &existing) {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    "refusing to replace director token: current token required".into(),
                ));
            }
        }
        let _ = crate::workspace::setup::persist_config(dir, &input.name, director_token);
        // Persist the new fields (director_name, experience_level, api_key) — merge
        // into whatever's already on disk so the telegram token is preserved.
        let _ = crate::workspace::setup::persist_setup_prefs(
            dir,
            input.director_name.as_deref(),
            input.experience_level.as_deref(),
            input.api_key.as_deref(),
            input.provider.as_deref(),
            input.model.as_deref(),
        );
    }

    // Fire the director's objective so onboarding produces the build plan.
    let ev = Event::new(
        &state.project,
        Actor::Director {
            user_id: "ceo".into(),
        },
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
