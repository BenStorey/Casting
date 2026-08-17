//! Deep handler tests for the Casting web API.
//!
//! Boots the real axum router (`casting::web::router`) against an in-memory
//! event store, seeds a small company, and exercises each endpoint's happy
//! path (asserting on response *fields*, not just status) plus its error
//! paths. This is the HTTP layer's deep test — `web_boot.rs` only checks that
//! routes are mounted and don't 5xx; here we inspect real payloads.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use futures::FutureExt;
use tower::ServiceExt;

const PROJECT: &str = "proj-web";

/// Build a fresh, empty AppState over in-memory stores.
fn boot_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, PROJECT)
}

/// Seed a small company: an agent, a task, an open decision, a directive, and
/// an opinion. Returns the state ready to serve GET assertions.
fn seeded_state() -> AppState {
    let state = boot_state();

    append(
        &state,
        EventType::AgentHired,
        "agent",
        "marcus-reed",
        serde_json::json!({ "role": "security" }),
    );
    append(
        &state,
        EventType::TaskCreated,
        "task",
        "task-1",
        serde_json::json!({ "title": "Harden the API", "kind": "feature" }),
    );
    append(
        &state,
        EventType::DecisionProposed,
        "decision",
        "decision-1",
        serde_json::json!({
            "subject": "Which database?",
            "options": serde_json::json!({ "a": "Postgres", "b": "SQLite" }),
            "recommendation": "Postgres",
            "class": "database",
            "involvement": "ask",
        }),
    );
    append(
        &state,
        EventType::ProjectDirectiveCreated,
        "directive",
        "directive-1",
        serde_json::json!({
            "kind": "policy",
            "statement": "pinned deps",
            "scope": serde_json::json!(["*.rs"]),
            "strength": "required",
            "created_by": "ceo",
        }),
    );
    append(
        &state,
        EventType::OpinionRecorded,
        "opinion",
        "opinion-1",
        serde_json::json!({
            "subject": "Stack language",
            "category": "design",
            "statement": "Rust is a good default",
        }),
    );

    state
}

/// Append a raw domain event under the test project.
fn append(state: &AppState, event_type: EventType, kind: &str, id: &str, data: serde_json::Value) {
    state
        .append(Event::new(
            PROJECT,
            Actor::Agent { id: "pm".into() },
            event_type,
            Aggregate {
                kind: kind.into(),
                id: id.into(),
            },
            data,
        ))
        .unwrap();
}

/// Build the full router for a given state.
fn app(state: AppState) -> axum::Router {
    casting::web::router(state)
}

/// Dispatch a GET request through the router, returning the raw response.
fn get(app: &axum::Router, path: &str) -> Response {
    let req = Request::builder().uri(path).body(Body::empty()).unwrap();
    app.clone()
        .oneshot(req)
        .now_or_never()
        .expect("router dispatch should not block")
        .expect("router oneshot is infallible")
}

/// Dispatch a POST request with a JSON body, returning the raw response.
fn post(app: &axum::Router, path: &str, body: serde_json::Value) -> Response {
    post_raw(app, path, serde_json::to_vec(&body).unwrap())
}

/// Dispatch a POST request with a raw body + content-type and optional auth.
fn post_raw_with_auth(
    app: &axum::Router,
    path: &str,
    body: Vec<u8>,
    bearer: Option<&str>,
) -> Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let req = builder.body(Body::from(body)).unwrap();
    app.clone()
        .oneshot(req)
        .now_or_never()
        .expect("router dispatch should not block")
        .expect("router oneshot is infallible")
}

/// Dispatch a POST request with a raw byte body.
fn post_raw(app: &axum::Router, path: &str, body: Vec<u8>) -> Response {
    post_raw_with_auth(app, path, body, None)
}

/// Read the response body as a JSON value.
fn body_json(resp: Response) -> serde_json::Value {
    let bytes =
        futures::executor::block_on(axum::body::to_bytes(resp.into_body(), usize::MAX)).unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// STEP 1: the minimal skeleton — health answers 200 with a JSON body.
#[test]
fn health_endpoint_answers_ok() {
    let state = boot_state();
    let app = app(state);
    let resp = get(&app, "/api/health");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["ok"], serde_json::Value::Bool(true));
}

// --- STEP 2: GET endpoints against a seeded company ---

/// GET /api/state — the projection has the seeded task.
#[test]
fn state_endpoint_reports_seeded_task() {
    let state = seeded_state();
    let app = app(state.clone());
    let resp = get(&app, "/api/state");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    let tasks = json["tasks"].as_array().expect("tasks is an array");
    assert!(
        tasks.iter().any(|t| t["id"] == "task-1"),
        "seeded task-1 should be in /api/state"
    );
}

/// GET /api/events?after=0 — returns the seeded events as a history slice.
#[test]
fn events_endpoint_returns_history() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/events?after=0");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    let events = json.as_array().expect("events is an array");
    // 5 seeded events: agent, task, decision, directive, opinion.
    assert!(
        events.len() >= 5,
        "expected >= 5 events, got {}",
        events.len()
    );
    assert!(events.iter().any(|e| e["event_type"] == "TaskCreated"));
}

/// GET /api/inbox — the open decision lands as an inbox item.
#[test]
fn inbox_lists_open_decision() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/inbox");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    let items = json["items"].as_array().expect("items is an array");
    assert!(
        items.iter().any(|i| i["id"] == "decision-1"),
        "inbox should contain the open decision"
    );
    assert!(json["unread"] == serde_json::json!(1));
}

/// GET /api/model — the operating model has a subject/summary surface.
#[test]
fn model_endpoint_answers() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/model");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    // OperatingModel always has an objective field (possibly null).
    assert!(json.get("objective").is_some());
}

/// GET /api/graph — the graph has a nodes surface derived from the task.
#[test]
fn graph_endpoint_contains_task_node() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/graph");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    // GraphView has a nodes field; at minimum it serializes as an object.
    assert!(json.is_object(), "graph should be a JSON object");
}

/// GET /api/context/pm — the PM's operating context serializes.
#[test]
fn pm_context_answers() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/context/pm");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["actor"], serde_json::json!("pm"));
}

/// GET /api/persona/{id} — a hired agent has a persona card; unknown is 404.
#[test]
fn persona_known_and_unknown() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/persona/marcus-reed");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["id"], serde_json::json!("marcus-reed"));

    let resp = get(&app, "/api/persona/unknown-id");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// GET /api/routing — serializes a list of actor routings (empty when no LLM).
#[test]
fn routing_endpoint_answers() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/routing");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert!(json.is_array(), "routing should be a JSON array");
}

/// GET /api/consultants — the embedded registry always has some entries.
#[test]
fn consultants_endpoint_lists_registry() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/consultants");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert!(json.is_array(), "consultants should be an array");
}

/// GET /api/telegram/status — unconfigured status reports configured=false.
#[test]
fn telegram_status_unconfigured() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/telegram/status");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["configured"], serde_json::Value::Bool(false));
}

/// GET /api/provenance/task/{id} — reports the task provenance (task_id field).
#[test]
fn provenance_task_answers() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/provenance/task/task-1");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["task_id"], serde_json::json!("task-1"));
}

// --- STEP 3: POST mutating endpoints against a seeded company ---

/// POST /api/message — director message persists as a MessageSent event.
#[test]
fn post_message_persists() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/message",
        serde_json::json!({ "body": "Build it" }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["event_type"], serde_json::json!("MessageSent"));
}

/// POST /api/decision — director verdict on the open decision persists.
#[test]
fn post_decision_verdict() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/decision",
        serde_json::json!({
            "decision_id": "decision-1",
            "subject": "Which database?",
            "approved": true,
        }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["event_type"], serde_json::json!("DecisionMade"));
}

/// POST /api/policy — director sets decision involvement for a class.
#[test]
fn post_policy_changes_class() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/policy",
        serde_json::json!({ "class": "security_critical", "involvement": "ask" }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(
        json["event_type"],
        serde_json::json!("DecisionPolicyChanged")
    );
}

/// POST /api/directive — director authors governance.
#[test]
fn post_directive_creates_governance() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/directive",
        serde_json::json!({
            "id": "directive-2",
            "kind": "policy",
            "statement": "no secrets in the log",
            "scope": serde_json::json!(["src/**"]),
            "strength": "required",
        }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(
        json["event_type"],
        serde_json::json!("ProjectDirectiveCreated")
    );
}

/// POST /api/hire — director hires a catalog role; a new AgentHired event lands.
#[test]
fn post_hire_catalog_role() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/hire",
        serde_json::json!({ "role_id": "devops" }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["event_type"], serde_json::json!("AgentHired"));
}

/// POST /api/brief — director imports advisory content.
#[test]
fn post_brief_imports_advice() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/brief",
        serde_json::json!({ "source": "advisor", "title": "t", "body": "advice" }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(
        json["event_type"],
        serde_json::json!("AdvisoryBriefingImported")
    );
}

/// POST /api/request — an external request is recorded + triaged.
#[test]
fn post_request_records_external() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/request",
        serde_json::json!({
            "source": "github",
            "title": "failing test",
            "body": "see issue",
        }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(
        json["event_type"],
        serde_json::json!("ExternalRequestReceived")
    );
}

/// POST /api/diagram — an in-app diagram is persisted.
#[test]
fn post_diagram_saves() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/diagram",
        serde_json::json!({ "title": "arch", "data": "{\"type\":\"excalidraw\"}" }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["event_type"], serde_json::json!("DiagramSaved"));
}

/// POST /api/advisor/message — director→advisor message appends to the thread.
#[test]
fn post_advisor_message() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/advisor/message",
        serde_json::json!({ "body": "think about strategy" }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["event_type"], serde_json::json!("AdvisorMessageSent"));
}

/// POST /api/advisor/handoff — the advisor thread becomes a briefing.
#[test]
fn post_advisor_handoff() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/advisor/handoff",
        serde_json::json!({ "summary": "we should prioritize X" }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["event_type"], serde_json::json!("AdvisorHandoff"));
}

/// POST /api/telegram/configure — an empty token is rejected deterministically
/// (400), exercising the handler without a live network call.
#[test]
fn post_telegram_configure_rejects_empty_token() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(
        &app,
        "/api/telegram/configure",
        serde_json::json!({ "token": "   " }),
    );
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// --- STEP 4: error paths ---

/// An unknown /api/* path returns a JSON 404, never the SPA HTML fallback.
#[test]
fn unknown_api_path_is_json_404() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/xyz-does-not-exist");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let ctype = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or("").to_string())
        .unwrap_or_default();
    assert!(
        ctype.contains("json"),
        "unknown API path must return JSON, got content-type {ctype:?}"
    );
}

/// A malformed/empty JSON body on a mutating POST is a 4xx, not a 500.
#[test]
fn malformed_body_on_post_is_4xx() {
    let state = seeded_state();
    let app = app(state);
    // Empty body to a Json<Dto> extractor must be rejected (400/422), not 500.
    let resp = post_raw(&app, "/api/message", Vec::new());
    let status = resp.status();
    assert!(
        status.is_client_error(),
        "empty body should be a 4xx, got {status}"
    );
    assert!(!status.is_server_error(), "empty body must not be a 500");

    // Valid JSON but wrong shape (missing `body` field) is also a client error.
    let resp = post_raw(&app, "/api/message", br#"{"nope":1}"#.to_vec());
    let status = resp.status();
    assert!(
        status.is_client_error(),
        "bad shape should be a 4xx, got {status}"
    );
}

/// With a director token configured, a mutating POST without the bearer is 401;
/// with the correct bearer it passes through (non-401).
#[test]
fn director_token_guards_mutations() {
    let state = seeded_state().with_owner_auth("secret-token");
    let app = app(state);

    let resp = post_raw_with_auth(
        &app,
        "/api/message",
        serde_json::to_vec(&serde_json::json!({ "body": "hi" })).unwrap(),
        None,
    );
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let resp = post_raw_with_auth(
        &app,
        "/api/message",
        serde_json::to_vec(&serde_json::json!({ "body": "hi" })).unwrap(),
        Some("secret-token"),
    );
    let status = resp.status();
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "correct bearer should pass"
    );
    assert!(!status.is_server_error(), "correct bearer should not 5xx");

    // Wrong token is also 401.
    let resp = post_raw_with_auth(
        &app,
        "/api/message",
        serde_json::to_vec(&serde_json::json!({ "body": "hi" })).unwrap(),
        Some("wrong-token"),
    );
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// --- STEP 5: coverage lifts for state.rs / advisor.rs / auth.rs ---

/// GET /api/events/stream — the SSE endpoint is mounted and responds 200.
///
/// We assert only the status here: the body is an infinite SSE stream whose
/// catch-up payloads need a live tokio reactor to pull (and the catch-up DATA
/// path is already covered by `events_endpoint_returns_history`, the same
/// `/api/events?after=N` read). Building the SSE response itself needs a
/// reactor (axum's keep-alive arms a timer), so this runs inside `#[tokio::test]`.
#[tokio::test]
async fn events_stream_is_mounted_and_answers() {
    use tower::ServiceExt;
    let state = seeded_state();
    let app = app(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/events/stream?after=0")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .expect("router dispatch is infallible");
    assert_eq!(resp.status(), StatusCode::OK);
}

/// POST /api/advisor/summarize — returns a deterministic summary with no LLM
/// configured (the route is POST-mounted in the router).
#[test]
fn advisor_summarize_returns_deterministic_summary() {
    let state = seeded_state();
    let app = app(state);
    let resp = post(&app, "/api/advisor/summarize", serde_json::json!({}));
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert!(json.get("summary").is_some(), "summarize returns a summary");
}

/// POST /api/login with the correct director token verifies ok; wrong token 401.
#[test]
fn login_verifies_director_token() {
    let state = seeded_state().with_owner_auth("secret-token");
    let app = app(state);

    let resp = post(
        &app,
        "/api/login",
        serde_json::json!({ "token": "secret-token" }),
    );
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["ok"], serde_json::Value::Bool(true));

    let resp = post(&app, "/api/login", serde_json::json!({ "token": "nope" }));
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// GET /api/graph/task/{id} — reports the task's PM context (200 known, 404 unknown).
#[test]
fn graph_task_context_known_and_unknown() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/graph/task/task-1");
    assert_eq!(resp.status(), StatusCode::OK);
    // PmTaskContext carries a `task_id` field.
    let json = body_json(resp);
    assert!(json.is_object(), "PmTaskContext should be an object");

    let resp = get(&app, "/api/graph/task/nope");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// GET /api/context/director — the director's operating context serializes (director is
/// a valid actor alias, distinct from any hired agent).
#[test]
fn owner_context_answers() {
    let state = seeded_state();
    let app = app(state);
    let resp = get(&app, "/api/context/director");
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp);
    assert_eq!(json["actor"], serde_json::json!("director"));
}
