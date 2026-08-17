use super::{append_json, CurrentUser};
use crate::event::Event;
use crate::pm::AppState;
use crate::workspace::secrets::ensure_no_secrets_in_text;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct DecisionIn {
    decision_id: String,
    subject: String,
    approved: bool,
    #[serde(default)]
    note: Option<String>,
}

/// POST /api/decision — the director records a verdict on a proposed decision.
/// Durable `DecisionMade`; the PM loop reacts and drives follow-up.
pub(crate) async fn decision_handler(
    State(state): State<AppState>,
    CurrentUser(user_id): CurrentUser,
    Json(input): Json<DecisionIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    // Shape is owned by actions.rs so it can never drift from to_events.
    let ev = crate::actions::director_decision_made(
        &user_id,
        &state.project,
        &input.decision_id,
        &input.subject,
        input.approved,
        input.note.clone(),
    );
    // Reject if decision subject/note embeds a raw secret value
    if let Some(ref secrets) = state.secrets {
        ensure_no_secrets_in_text(secrets, &input.subject, "decision subject")
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        if let Some(ref note) = input.note {
            ensure_no_secrets_in_text(secrets, note, "decision note")
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        }
    }
    append_json(&state, ev)
}

#[derive(Deserialize)]
pub(crate) struct PolicyIn {
    class: crate::pm::DecisionClass,
    involvement: crate::pm::OwnerInvolvement,
}

/// POST /api/policy — the director sets the director-involvement for a decision
/// class (delegated authority, brief §5). Durable `DecisionPolicyChanged`; the
/// projection folds it into the event-sourced policy that the gate enforces.
pub(crate) async fn policy_handler(
    State(state): State<AppState>,
    CurrentUser(user_id): CurrentUser,
    Json(input): Json<PolicyIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let ev = crate::actions::director_policy_changed(
        &user_id,
        &state.project,
        input.class,
        input.involvement,
    );
    append_json(&state, ev)
}

#[derive(Deserialize)]
pub(crate) struct DirectiveIn {
    id: String,
    kind: crate::runtime::directive::DirectiveKind,
    statement: String,
    #[serde(default)]
    scope: Vec<String>,
    #[serde(default = "default_strength")]
    strength: crate::runtime::directive::DirectiveStrength,
}

fn default_strength() -> crate::runtime::directive::DirectiveStrength {
    crate::runtime::directive::DirectiveStrength::Required
}

/// POST /api/directive — the DIRECTOR sets project governance (docs/INTENT.md).
/// Only the director may author directives; this endpoint is the director's surface
/// (mirrors /api/policy). Durable `ProjectDirectiveCreated`.
pub(crate) async fn directive_handler(
    State(state): State<AppState>,
    CurrentUser(user_id): CurrentUser,
    Json(input): Json<DirectiveIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let ev = crate::actions::director_directive_created(
        &user_id,
        &state.project,
        &input.id,
        input.kind,
        &input.statement,
        input.scope,
        input.strength,
    );
    append_json(&state, ev)
}

/// POST /api/hire — the DIRECTOR adds an agent of a curated role to the cast.
#[derive(Deserialize)]
pub(crate) struct HireIn {
    /// A role id from the role catalog (e.g. "security", "devops").
    role_id: String,
}

/// POST /api/hire — the DIRECTOR adds an agent of a role to the cast (delegated
/// authority: the CEO grows the team). Resolves the role against the dynamic
/// role set — the catalog PLUS any roles the loaded consultants fill via
/// `cast_role` — so a director can hire a custom consultant. It
/// then generates a unique agent id and persists `AgentHired`
/// via the validated `HireAgent` action.
pub(crate) async fn hire_handler(
    State(state): State<AppState>,
    CurrentUser(user_id): CurrentUser,
    Json(input): Json<HireIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    // The role must be known: a catalog role OR one a consultant package defined.
    let role = state
        .consultants
        .resolve_role(&input.role_id)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!("unknown role {:?}", input.role_id),
            )
        })?;

    // Unique agent id: role id + a monotonic counter of existing agents.
    let proj = state
        .projection()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let taken = proj
        .agents
        .iter()
        .map(|a| a.id.as_str())
        .collect::<Vec<_>>();
    let mut n = 1;
    let agent_id = loop {
        let candidate = format!("{}-{n}", input.role_id);
        if !taken.contains(&candidate.as_str()) {
            break candidate;
        }
        n += 1;
    };

    // Route through the validated HireAgent action (director authority).
    let action = crate::actions::PmAction::HireAgent {
        agent_id: agent_id.clone(),
        role: role.title.to_string(),
    };
    if let Err(e) = crate::actions::validate(&action, "director", &proj, None) {
        return Err((StatusCode::CONFLICT, e.to_string()));
    }
    let cause = Event::new(
        &state.project,
        crate::event::Actor::Director {
            user_id: user_id.clone(),
        },
        crate::event::EventType::MessageSent,
        crate::event::Aggregate {
            kind: "message".into(),
            id: format!("msg-hire-{agent_id}"),
        },
        serde_json::json!({ "to": "pm", "body": "hiring" }),
    );
    let last = action
        .to_events(&state.project, "director", &cause, "hire")
        .into_iter()
        .map(|e| state.append(e))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .pop()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "HireAgent produced no events".into(),
            )
        })?;
    Ok(Json(last))
}

// --- Harness guards (2026-08-13, docs/plans/2026-08-13_harness-guards.md) ---

#[derive(Deserialize)]
pub(crate) struct BudgetIn {
    limit_usd: f64,
    #[serde(default)]
    warn_at: Option<f64>,
}

/// POST /api/budget — the DIRECTOR sets the hard spend circuit breaker. Once set,
/// the dispatch gate refuses LLM calls when `total_spend >= limit_usd` (and
/// warns at `warn_at * limit_usd`). Durable `BudgetSet`. The
/// breaker is OUTSIDE the PM's control by construction.
pub(crate) async fn budget_handler(
    State(state): State<AppState>,
    CurrentUser(user_id): CurrentUser,
    Json(input): Json<BudgetIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let warn_at = input.warn_at.unwrap_or(0.80);
    let ev =
        crate::actions::director_budget_set(&user_id, &state.project, input.limit_usd, warn_at);
    append_json(&state, ev)
}

#[derive(Deserialize)]
pub(crate) struct PauseIn {
    reason: String,
}

/// POST /api/pause — the DIRECTOR pauses all side-effecting work (resumable via
/// /api/resume). The liveness watchdog issues the same `WorkPaused` internally.
pub(crate) async fn pause_handler(
    State(state): State<AppState>,
    CurrentUser(user_id): CurrentUser,
    Json(input): Json<PauseIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let ev = crate::actions::director_work_paused(&user_id, &state.project, &input.reason);
    append_json(&state, ev)
}

/// POST /api/resume — the DIRECTOR clears a `WorkPaused`. A BUDGET halt (derived
/// from spend) is NOT cleared by this — only a higher budget limit un-halts it.
pub(crate) async fn resume_handler(
    State(state): State<AppState>,
    CurrentUser(user_id): CurrentUser,
) -> Result<Json<Event>, (StatusCode, String)> {
    let ev = crate::actions::director_work_resumed(&user_id, &state.project);
    append_json(&state, ev)
}
