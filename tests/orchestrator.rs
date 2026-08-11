//! Tests for the D2 orchestrator seam (docs/PLAN: A — mocked provider).
//!
//! The real LLM is deliberately UNPLUGGED (off by default, no spend). The
//! MockOrchestrator proves the seam end-to-end: context -> PmActions -> gate ->
//! events, with zero live model.

use casting::cursor::CursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::orchestrator::MockOrchestrator;
use casting::pm::AppState;
use casting::projection::Projection;
use casting::sqlite_store::SqliteEventStore;
use std::sync::Arc;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = CursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-orch")
}

#[test]
fn orchestrator_is_off_by_default() {
    let state = make_state();
    assert!(
        state.orchestrator.is_none(),
        "real LLM must be unplugged by default"
    );
}

#[tokio::test]
async fn mock_orchestrator_drives_the_pm_loop_end_to_end() {
    let state = make_state().with_orchestrator(Arc::new(MockOrchestrator));
    // Seed the project + hire the PM (what main.rs does on startup).
    state
        .append(Event::new(
            "proj-orch",
            Actor::System,
            EventType::ProjectCreated,
            Aggregate {
                kind: "project".into(),
                id: "proj-orch".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-orch",
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: "pm".into(),
            },
            serde_json::json!({ "role": "Project Manager" }),
        ))
        .unwrap();

    // Owner sends the first message.
    state
        .append(Event::new(
            "proj-orch",
            Actor::Owner,
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-1".into(),
            },
            serde_json::json!({ "body": "Build me a thing" }),
        ))
        .unwrap();

    // Drive the PM: the mock orchestrator responds (not the scripted plan).
    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-orch").unwrap();
    // The mock acknowledged the owner with a MessageSent from PM.
    assert!(
        proj.messages
            .iter()
            .any(|m| m.from == "pm" && m.to == "owner" && m.body.contains("Build me a thing")),
        "mock orchestrator should acknowledge the owner message"
    );

    // Now establish an objective (a requirement), then the next owner message
    // drives the mock to actually BUILD a task from context.
    state
        .append(Event::new(
            "proj-orch",
            Actor::Agent { id: "pm".into() },
            EventType::RequirementCreated,
            Aggregate {
                kind: "requirement".into(),
                id: "req-1".into(),
            },
            serde_json::json!({ "title": "Build me a thing", "description": "x" }),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-orch",
            Actor::Owner,
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-2".into(),
            },
            serde_json::json!({ "body": "go" }),
        ))
        .unwrap();
    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-orch").unwrap();
    assert!(
        proj.tasks.iter().any(|t| t.id == "task-mock-1"),
        "with an objective, the mock orchestrator plans a task from context"
    );
}
