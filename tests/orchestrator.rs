//! Tests for the orchestrator seam (D2) and its MockOrchestrator implementation.
//!
//! The mock is a stateless test double: it produces deterministic actions but
//! does NOT record metering/cost data (real cost tracking happens through the
//! LlmOrchestrator in production). Tests that need cost tracking should use
//! a real provider client or a different test seam.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::runtime::orchestrator::MockOrchestrator;
use casting::store::EventStore;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
use std::sync::Arc;
use std::time::Duration;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-orch")
        .with_step_delay(Duration::ZERO)
        .with_orchestrator(Arc::new(MockOrchestrator))
}

/// Seed a requirement so the mock has an objective (else it only acks).
fn seed_requirement(state: &AppState) {
    state
        .append(Event::new(
            "proj-orch",
            Actor::System,
            EventType::RequirementCreated,
            Aggregate {
                kind: "requirement".into(),
                id: "req-1".into(),
            },
            serde_json::json!({ "title": "Build a thing", "body": "Build a thing" }),
        ))
        .unwrap();
    state
        .cursors
        .advance(
            "proj-orch",
            "pm",
            state.store.latest_sequence("proj-orch").unwrap(),
        )
        .unwrap();
}

#[tokio::test]
async fn orchestrator_plans_actions_from_context() {
    let state = make_state();
    seed_requirement(&state);

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

    let authored = casting::pm::drive_pm(&state).await.unwrap();
    assert!(
        authored > 0,
        "mock should author at least one action per plan"
    );
}

#[tokio::test]
async fn mock_does_not_record_cost() {
    // The MockOrchestrator is a stateless test double — it does NOT
    // record metering/cost data. This test verifies that invariant.
    let state = make_state();
    seed_requirement(&state);
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

    casting::pm::drive_pm(&state).await.unwrap();
    let proj = Projection::build(&state.store, "proj-orch").unwrap();
    assert!(
        proj.spend.is_empty(),
        "a stateless mock must not record cost (got {} entries)",
        proj.spend.len()
    );
}

#[tokio::test]
async fn orchestrator_records_a_planning_run_in_diagnostics() {
    // The planning diagnostic (OrchestrationRun event) is always recorded,
    // regardless of metering. Only the metering-specific fields change.
    let state = make_state();
    seed_requirement(&state);
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

    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-orch").unwrap();
    assert_eq!(
        proj.orchestration.len(),
        1,
        "one orchestrator planning pass should be recorded"
    );
    let run = &proj.orchestration[0];
    assert_eq!(run.trigger, "MessageSent");
    assert_eq!(run.actor, "pm");
    assert!(
        run.context_summary.contains("objective="),
        "context summary tells the reader what was handed in: {}",
        run.context_summary
    );
    assert!(
        run.planned.iter().any(|p| p.contains("\"create_task\"")),
        "recorded what the model decided to do: {:?}",
        run.planned
    );
    // The mock is stateless — no metering is reported.
    assert!(!run.metered, "a stateless mock reports no metering");
    assert_eq!(run.provider, None);
    assert_eq!(run.estimated_usd, 0.0);
}
