use super::append_json;
use crate::event::Event;
use crate::pm::AppState;
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

/// POST /api/decision — the owner records a verdict on a proposed decision.
/// Durable `DecisionMade` (actor = Owner); the PM loop reacts and drives follow-up.
pub(crate) async fn decision_handler(
    State(state): State<AppState>,
    Json(input): Json<DecisionIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    // Shape is owned by actions.rs so it can never drift from to_events.
    let ev = crate::actions::owner_decision_made(
        &state.project,
        &input.decision_id,
        &input.subject,
        input.approved,
        input.note.clone(),
    );
    append_json(&state, ev)
}

#[derive(Deserialize)]
pub(crate) struct PolicyIn {
    class: crate::pm::DecisionClass,
    involvement: crate::pm::OwnerInvolvement,
}

/// POST /api/policy — the owner sets the owner-involvement for a decision
/// class (delegated authority, brief §5). Durable `DecisionPolicyChanged`; the
/// projection folds it into the event-sourced policy that the gate enforces.
pub(crate) async fn policy_handler(
    State(state): State<AppState>,
    Json(input): Json<PolicyIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let ev = crate::actions::owner_policy_changed(&state.project, input.class, input.involvement);
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

/// POST /api/directive — the OWNER sets project governance (docs/INTENT.md).
/// Only the owner may author directives; this endpoint is the owner's surface
/// (mirrors /api/policy). Durable `ProjectDirectiveCreated` (actor = Owner).
pub(crate) async fn directive_handler(
    State(state): State<AppState>,
    Json(input): Json<DirectiveIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let ev = crate::actions::owner_directive_created(
        &state.project,
        &input.id,
        input.kind,
        &input.statement,
        input.scope,
        input.strength,
    );
    append_json(&state, ev)
}

/// POST /api/hire — the OWNER adds an agent of a curated role to the cast.
#[derive(Deserialize)]
pub(crate) struct HireIn {
    /// A role id from the role catalog (e.g. "security", "devops").
    role_id: String,
}

/// POST /api/hire — the OWNER adds an agent of a role to the cast (delegated
/// authority: the CEO grows the team). Resolves the role against the dynamic
/// role set — the catalog PLUS any roles the loaded consultants fill via
/// `cast_role` — so an owner can hire a custom consultant. It
/// then generates a unique agent id and persists `AgentHired` (actor = Owner)
/// via the validated `HireAgent` action.
pub(crate) async fn hire_handler(
    State(state): State<AppState>,
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

    // Route through the validated HireAgent action (owner authority).
    let action = crate::actions::PmAction::HireAgent {
        agent_id: agent_id.clone(),
        role: role.title.to_string(),
    };
    if let Err(e) = crate::actions::validate(&action, "owner", &proj) {
        return Err((StatusCode::CONFLICT, e.to_string()));
    }
    let cause = Event::new(
        &state.project,
        crate::event::Actor::Owner,
        crate::event::EventType::MessageSent,
        crate::event::Aggregate {
            kind: "message".into(),
            id: format!("msg-hire-{agent_id}"),
        },
        serde_json::json!({ "to": "pm", "body": "hiring" }),
    );
    let last = action
        .to_events(&state.project, "owner", &cause, "hire")
        .into_iter()
        .map(|e| state.append(e))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .pop()
        .expect("HireAgent always produces one event");
    Ok(Json(last))
}

// --- Harness guards (2026-08-13, docs/plans/2026-08-13_harness-guards.md) ---

#[derive(Deserialize)]
pub(crate) struct BudgetIn {
    limit_usd: f64,
    #[serde(default)]
    warn_at: Option<f64>,
}

/// POST /api/budget — the OWNER sets the hard spend circuit breaker. Once set,
/// the dispatch gate refuses LLM calls when `total_spend >= limit_usd` (and
/// warns at `warn_at * limit_usd`). Durable `BudgetSet` (actor = Owner). The
/// breaker is OUTSIDE the PM's control by construction.
pub(crate) async fn budget_handler(
    State(state): State<AppState>,
    Json(input): Json<BudgetIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let warn_at = input.warn_at.unwrap_or(0.80);
    let ev = crate::actions::owner_budget_set(&state.project, input.limit_usd, warn_at);
    append_json(&state, ev)
}

#[derive(Deserialize)]
pub(crate) struct PauseIn {
    reason: String,
}

/// POST /api/pause — the OWNER pauses all side-effecting work (resumable via
/// /api/resume). The liveness watchdog issues the same `WorkPaused` internally.
pub(crate) async fn pause_handler(
    State(state): State<AppState>,
    Json(input): Json<PauseIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let ev = crate::actions::owner_work_paused(&state.project, &input.reason);
    append_json(&state, ev)
}

/// POST /api/resume — the OWNER clears a `WorkPaused`. A BUDGET halt (derived
/// from spend) is NOT cleared by this — only a higher budget limit un-halts it.
pub(crate) async fn resume_handler(
    State(state): State<AppState>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let ev = crate::actions::owner_work_resumed(&state.project);
    append_json(&state, ev)
}
