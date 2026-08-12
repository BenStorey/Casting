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
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::middleware::Next;
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
    class: String,
    involvement: String,
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

/// POST /api/brief input: external advisor content. `source` marks provenance
/// (e.g. "ChatGPT advisor") so it's never confusable with the owner's intent.
#[derive(Deserialize)]
struct BriefIn {
    source: Option<String>,
    subject: Option<String>,
    title: Option<String>,
    body: String,
    /// Optional image/diagram references (caption + path/URL).
    #[serde(default)]
    assets: Vec<crate::projection::BriefingAsset>,
}

/// POST /api/request input: an EXTERNAL request (e.g. a GitHub issue/PR a
/// product user opened). `source` + `external_id` + `reporter` record where it
/// came from so the PM can triage it. NOT the owner's own intent.
#[derive(Deserialize)]
struct RequestIn {
    /// e.g. "github" | "email" | "web".
    #[serde(default = "default_source")]
    source: String,
    external_id: Option<String>,
    title: String,
    #[serde(default)]
    body: String,
    /// Who raised it (e.g. GitHub username). Defaults to "external".
    #[serde(default = "default_reporter")]
    reporter: String,
    #[serde(default)]
    labels: Vec<String>,
    url: Option<String>,
}

fn default_source() -> String {
    "external".to_string()
}
fn default_reporter() -> String {
    "external".to_string()
}

#[derive(Deserialize)]
struct DecisionIn {
    decision_id: String,
    subject: String,
    approved: bool,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize)]
struct PolicyIn {
    class: crate::policy::DecisionClass,
    involvement: crate::policy::OwnerInvolvement,
}

#[derive(Deserialize)]
struct DirectiveIn {
    id: String,
    kind: crate::directive::DirectiveKind,
    statement: String,
    #[serde(default)]
    scope: Vec<String>,
    #[serde(default = "default_strength")]
    strength: crate::directive::DirectiveStrength,
}

fn default_strength() -> crate::directive::DirectiveStrength {
    crate::directive::DirectiveStrength::Required
}

/// POST /api/hire — the OWNER adds an agent of a curated role to the cast.
#[derive(Deserialize)]
struct HireIn {
    /// A role id from the role catalog (e.g. "security", "devops").
    role_id: String,
}

/// Build the full router for a project's runtime state.
pub fn router(state: AppState) -> Router {
    // Owner-mutating endpoints are bearer-guarded when auth is enabled (the
    // middleware consults AppState.auth_token; no-op when it's None).
    let guarded = Router::new()
        .route("/api/message", axum::routing::post(message_handler))
        .route("/api/brief", axum::routing::post(brief_handler))
        .route("/api/request", axum::routing::post(request_handler))
        .route("/api/decision", axum::routing::post(decision_handler))
        .route("/api/policy", axum::routing::post(policy_handler))
        .route("/api/directive", axum::routing::post(directive_handler))
        .route("/api/hire", axum::routing::post(hire_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_auth,
        ));

    Router::new()
        .route("/api/login", axum::routing::post(login_handler))
        .route("/api/setup/status", get(setup_status_handler))
        .route("/api/setup", axum::routing::post(setup_handler))
        .route("/api/state", get(state_handler))
        .route("/api/events", get(events_handler))
        .route("/api/events/stream", get(events_stream))
        .route("/api/inbox", get(inbox_handler))
        .merge(guarded)
        .route(
            "/api/provenance/commit/{sha}",
            get(provenance_commit_handler),
        )
        .route(
            "/api/provenance/task/{task_id}",
            get(provenance_task_handler),
        )
        .route(
            "/api/provenance/decision/{decision_id}",
            get(provenance_decision_handler),
        )
        .route("/api/context/{actor}", get(context_handler))
        .route("/api/persona/{agent_id}", get(persona_handler))
        .route("/api/model", get(model_handler))
        // The embedded SPA (and SPA route fallback) handles everything else.
        .fallback(static_handler)
        .with_state(state)
}

/// Auth middleware for owner-mutating endpoints: when `AppState.auth_token` is
/// set, require `Authorization: Bearer <token>`; otherwise pass through (auth
/// disabled, backward compatible with tests / local runs).
async fn require_auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(expected) = state.auth_token.clone() {
        if !crate::auth::authorized(req.headers(), &expected) {
            return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
        }
    }
    next.run(req).await
}

#[derive(Deserialize)]
struct LoginIn {
    token: String,
}

/// POST /api/login {token} — verify an owner token (200 ok) or not (401). Lets a
/// UI validate the token the user pasted before using it for mutations.
async fn login_handler(
    State(state): State<AppState>,
    Json(input): Json<LoginIn>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.auth_token.as_deref() {
        Some(expected) if crate::auth::authorized(&fake_headers_with(&input.token), expected) => {
            Ok(Json(serde_json::json!({ "ok": true })))
        }
        // If auth is disabled entirely, any token is accepted (nothing to guard).
        None => Ok(Json(serde_json::json!({ "ok": true }))),
        Some(_) => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Build a single-header map carrying a bearer token (used by login, which gets
/// the token in the body rather than the header).
fn fake_headers_with(token: &str) -> axum::http::HeaderMap {
    use axum::http::header::AUTHORIZATION;
    let mut m = axum::http::HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&format!("Bearer {token}")) {
        m.insert(AUTHORIZATION, v);
    }
    m
}

/// GET /api/setup/status — is this company configured, and what roles are
/// available to hire? The SPA shows a first-run wizard when `configured` is
/// false (i.e. no cast hired yet, only the seed PM).
async fn setup_status_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
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
struct SetupIn {
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
async fn setup_handler(
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

/// GET /api/state — the current projection (agents, tasks, decisions, ...).
async fn state_handler(State(state): State<AppState>) -> Result<Json<Projection>, StatusCode> {
    let proj = state
        .projection()
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

/// POST /api/brief — the owner imports EXTERNAL advisor content (text + optional
/// image/diagram refs) as an ADVISORY briefing. Explicitly advisory, NOT
/// authoritative: `source` records provenance, and it can inform context but
/// never sets rules (directives remain the only authority mechanism).
async fn brief_handler(
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
    crate::actions::validate(&action, "owner", &proj)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let cause = Event::new(
        &state.project,
        Actor::Owner,
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "bootstrap".into(),
        },
        serde_json::json!({}),
    );
    let ev = action
        .to_events(&state.project, "owner", &cause, "brief")
        .into_iter()
        .next()
        .expect("ImportBriefing produces one event");
    let stored = state
        .append(ev)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(stored))
}

/// POST /api/request — an EXTERNAL request (e.g. a GitHub issue/PR a user
/// opened). Recorded with provenance + deterministic triage; the PM can triage
/// it without it pretending to be the owner's own intent.
async fn request_handler(
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
    let action = crate::actions::PmAction::ReceiveExternalRequest {
        id: format!("req-{}", uuid::Uuid::new_v4()),
        source: input.source,
        external_id: input.external_id,
        title,
        body: input.body,
        reporter: input.reporter,
        labels: input.labels,
        url: input.url,
    };
    let proj = state
        .projection()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::actions::validate(&action, "pm", &proj)
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
        .to_events(&state.project, "pm", &cause, "request")
        .into_iter()
        .next()
        .expect("ReceiveExternalRequest produces one event");
    let stored = state
        .append(ev)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(stored))
}

/// POST /api/decision — the owner records a verdict on a proposed decision.
/// Durable `DecisionMade` (actor = Owner); the PM loop reacts and drives follow-up.
async fn decision_handler(
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
    let stored = state
        .append(ev)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(stored))
}

/// POST /api/policy — the owner sets the owner-involvement for a decision
/// class (delegated authority, brief §5). Durable `DecisionPolicyChanged`; the
/// projection folds it into the event-sourced policy that the gate enforces.
async fn policy_handler(
    State(state): State<AppState>,
    Json(input): Json<PolicyIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let ev = crate::actions::owner_policy_changed(&state.project, input.class, input.involvement);
    let stored = state
        .append(ev)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(stored))
}

/// POST /api/directive — the OWNER sets project governance (docs/INTENT.md).
/// Only the owner may author directives; this endpoint is the owner's surface
/// (mirrors /api/policy). Durable `ProjectDirectiveCreated` (actor = Owner).
async fn directive_handler(
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
    let stored = state
        .append(ev)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(stored))
}

/// POST /api/hire — the OWNER adds an agent of a curated role to the cast
/// (delegated authority: the CEO grows the team). Validates the role exists in
/// the catalog, generates a unique agent id, and persists `AgentHired` (actor =
/// Owner) via the validated `HireAgent` action.
async fn hire_handler(
    State(state): State<AppState>,
    Json(input): Json<HireIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    // The role must be a real catalog role.
    let role = crate::cast::role_by_id(&input.role_id).ok_or_else(|| {
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

/// GET /api/provenance/decision/{id} — the audit for a decision: who proposed
/// it, what class/involvement, who decided it, and why (to the owner's message).
async fn provenance_decision_handler(
    State(state): State<AppState>,
    axum::extract::Path(decision_id): axum::extract::Path<String>,
) -> Result<Json<provenance::DecisionAudit>, StatusCode> {
    provenance::for_decision(&state.store, &state.project, &decision_id)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/context/{actor} — the assembled operating context for an actor
/// (agent id, "owner", or "pm"): objective, priorities, their tasks, the
/// governance directives that apply to them, risks, and open decisions.
async fn context_handler(
    State(state): State<AppState>,
    axum::extract::Path(actor): axum::extract::Path<String>,
) -> Result<Json<crate::context::AgentContext>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(proj.context_for(&actor)))
}

/// GET /api/persona/{agent_id} — the derived persona/CV card for a hired agent.
async fn persona_handler(
    State(state): State<AppState>,
    axum::extract::Path(agent_id): axum::extract::Path<String>,
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
async fn model_handler(
    State(state): State<AppState>,
) -> Result<Json<crate::mental::OperatingModel>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(proj.operating_model()))
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
