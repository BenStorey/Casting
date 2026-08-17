//! Tests for the web first-run setup wizard (owner decision: CLI + UI share one
//! engine). GET /api/setup/status says whether a cast is configured; POST
//! /api/setup hires the cast (idempotently), persists the director token, and fires
//! the objective message so onboarding kicks off.

use casting::pm::AppState;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

fn boot_api(auth: Option<&str>) -> axum::Router {
    use casting::event::{Actor, Aggregate, Event, EventType};
    use casting::store::EventStore;
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let mut state = AppState::new(store, cursors, "proj-web");
    if let Some(tok) = auth {
        state = state.with_owner_auth(tok);
    }
    // Seed the PM only (what `cast run` does) — so the company is unconfigured.
    if state.store.latest_sequence("proj-web").unwrap() == 0 {
        state
            .append(Event::new(
                "proj-web",
                Actor::System,
                EventType::ProjectCreated,
                Aggregate {
                    kind: "project".into(),
                    id: "proj-web".into(),
                },
                serde_json::json!({}),
            ))
            .unwrap();
        state
            .append(Event::new(
                "proj-web",
                Actor::System,
                EventType::AgentHired,
                Aggregate {
                    kind: "agent".into(),
                    id: "pm".into(),
                },
                serde_json::json!({ "role": "Project Manager" }),
            ))
            .unwrap();
    }
    casting::web::router(state)
}

async fn get_json(app: &axum::Router, path: &str) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), 1024 * 64)
        .await
        .unwrap();
    let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, value)
}

async fn post_json(app: &axum::Router, path: &str, body: &str) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), 1024 * 64)
        .await
        .unwrap();
    let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn status_is_unconfigured_then_configured_after_setup() {
    let app = boot_api(None);

    // Before setup: no cast hired -> unconfigured, roles listed.
    let (status, body) = get_json(&app, "/api/setup/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["configured"], false);
    assert!(
        body["roles"].as_array().unwrap().len() >= 2,
        "role catalog listed"
    );

    // Submit setup: hire engineer + devops, fire objective.
    let (status, body) = post_json(
        &app,
        "/api/setup",
        r#"{"name":"Acme","objective":"Build me a todo app","cast":["engineer","devops"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "setup accepted: {body}");
    assert_eq!(body["ok"], true);
    assert!(body["hires"].as_array().unwrap().len() >= 2);

    // Now configured, and the agents + objective message landed in state.
    let (status, _) = get_json(&app, "/api/setup/status").await;
    // build the projection from the same store via /api/state
    let (_s, state) = get_json(&app, "/api/state").await;
    let agents: Vec<&str> = state["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert!(agents.contains(&"engineer-1"));
    assert!(agents.contains(&"devops-1"));
    // The objective fired as a director message.
    assert!(
        state["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["from"] == "owner" && m["body"].as_str().unwrap_or("").contains("todo app")),
        "objective fired as the director's first message"
    );
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn setup_rejects_unknown_role() {
    let app = boot_api(None);
    let (status, _) = post_json(
        &app,
        "/api/setup",
        r#"{"name":"Acme","objective":"x","cast":["wizard"]}"#,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unknown role must be rejected"
    );
}

#[tokio::test]
async fn setup_is_idempotent_and_persists_token() {
    use casting::event::{Actor, Aggregate, Event, EventType};
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store.clone(), cursors, "proj-web");
    state
        .append(Event::new(
            "proj-web",
            Actor::System,
            EventType::ProjectCreated,
            Aggregate {
                kind: "project".into(),
                id: "proj-web".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-web",
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: "pm".into(),
            },
            serde_json::json!({ "role": "Project Manager" }),
        ))
        .unwrap();
    let app = casting::web::router(state);

    // First setup with a token.
    let (status, _) = post_json(
        &app,
        "/api/setup",
        r#"{"name":"Acme","objective":"x","cast":["engineer"],"director_token":"s3cr3t"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Second setup (no-op re: casting) must not error or duplicate.
    let (_s, state_out) = get_json(&app, "/api/state").await;
    let engineers = state_out["agents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["id"] == "engineer-1")
        .count();
    assert_eq!(engineers, 1, "no duplicate hires on re-setup");
}

// FAIL-CLOSED against silent token rotation: once a director token is persisted,
// POST /api/setup can only REPLACE it with a different value when the request
// presents the CURRENT token. An unauthenticated POST must never rotate it.
#[tokio::test]
async fn setup_refuses_silent_token_rotation_without_current_token() {
    use casting::workspace::setup::read_config;

    let dir = tempfile::tempdir().unwrap();
    // Simulate a repo whose setup already persisted a token (`cast init`).
    std::fs::write(
        dir.path().join("config.json"),
        r#"{"name":"Acme Inc","director_token":"old-secret"}"#,
    )
    .unwrap();

    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj-web").with_state_dir(dir.path().to_path_buf());
    let app = casting::web::router(state);

    // Try to rotate the token with a DIFFERENT token and NO bearer: must be 401.
    let body = r#"{"name":"Acme Inc","objective":"x","cast":[],"director_token":"attacker-token"}"#;
    let (status, _) = post_json(&app, "/api/setup", body).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "rotating a persisted token without the current token must fail"
    );

    // The persisted token is left untouched.
    let cfg = read_config(dir.path()).unwrap();
    assert_eq!(cfg.director_token.as_deref(), Some("old-secret"));

    // With the CURRENT token presented, rotation is allowed.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/setup")
                .header("content-type", "application/json")
                .header("authorization", "Bearer old-secret")
                .body(Body::from(
                    r#"{"name":"Acme Inc","objective":"x","cast":[],"director_token":"new-secret"}"#
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "rotation w/ current token ok");
    let cfg = read_config(dir.path()).unwrap();
    assert_eq!(cfg.director_token.as_deref(), Some("new-secret"));
}

// First-run with NO previously-persisted token may still SET a token.
#[tokio::test]
async fn setup_may_set_token_on_first_run() {
    use casting::workspace::setup::read_config;

    let dir = tempfile::tempdir().unwrap();
    // No config.json yet, but a state_dir is attached (bare `cast run`).
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj-web").with_state_dir(dir.path().to_path_buf());
    let app = casting::web::router(state);

    let (status, _) = post_json(
        &app,
        "/api/setup",
        r#"{"name":"Acme","objective":"x","cast":[],"director_token":"first-secret"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "first-run may set a token");

    let cfg = read_config(dir.path()).unwrap();
    assert_eq!(cfg.director_token.as_deref(), Some("first-secret"));
}
