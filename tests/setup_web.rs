//! Tests for the web first-run setup wizard (owner decision: CLI + UI share one
//! engine). GET /api/setup/status says whether a cast is configured; POST
//! /api/setup hires the cast (idempotently), persists the owner token, and fires
//! the objective message so onboarding kicks off.

use casting::cursor::CursorStore;
use casting::pm::AppState;
use casting::sqlite_store::SqliteEventStore;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

fn boot_api(auth: Option<&str>) -> axum::Router {
    use casting::event::{Actor, Aggregate, Event, EventType};
    use casting::store::EventStore;
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = CursorStore::in_memory().unwrap();
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
    assert!(agents.contains(&"marcus-reed"));
    assert!(agents.contains(&"devops-1"));
    // The objective fired as an owner message.
    assert!(
        state["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["from"] == "owner" && m["body"].as_str().unwrap_or("").contains("todo app")),
        "objective fired as the owner's first message"
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
    let cursors = CursorStore::in_memory().unwrap();
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
        r#"{"name":"Acme","objective":"x","cast":["engineer"],"owner_token":"s3cr3t"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Second setup (no-op re: casting) must not error or duplicate.
    let (_s, state_out) = get_json(&app, "/api/state").await;
    let engineers = state_out["agents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["id"] == "marcus-reed")
        .count();
    assert_eq!(engineers, 1, "no duplicate hires on re-setup");
}
