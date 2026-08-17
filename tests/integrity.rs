//! Tests for write-time event-stream integrity (option B / roadmap hardening).
//!
//! When `with_integrity()` is enabled, `AppState::append` rejects events whose
//! precondition is missing (a DecisionMade with no prior DecisionProposed; a
//! TaskCompleted with no prior TaskCreated). Opt-in, so bare-event fixtures
//! still work when off.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-int")
}

fn event(et: EventType, id: &str, kind: &str) -> Event {
    Event::new(
        "proj-int",
        Actor::Agent { id: "mei".into() },
        et,
        Aggregate {
            kind: kind.into(),
            id: id.into(),
        },
        serde_json::json!({}),
    )
}

#[test]
fn without_integrity_bare_decision_made_is_allowed() {
    // Default state (integrity off): fixtures may hand-append bare events.
    let state = make_state();
    state
        .append(event(EventType::DecisionMade, "d-1", "decision"))
        .unwrap();
}

#[test]
fn with_integrity_orphan_decision_made_is_rejected() {
    let state = make_state().with_integrity();
    // No prior DecisionProposed for d-1 -> must be rejected at append.
    let err = state.append(event(EventType::DecisionMade, "d-1", "decision"));
    assert!(err.is_err(), "orphan DecisionMade must be rejected");
}

#[test]
fn with_integrity_valid_proposed_then_made_passes() {
    let state = make_state().with_integrity();
    state
        .append(event(EventType::DecisionProposed, "d-1", "decision"))
        .unwrap();
    state
        .append(event(EventType::DecisionMade, "d-1", "decision"))
        .unwrap();
}

#[test]
fn with_integrity_orphan_task_completed_is_rejected() {
    let state = make_state().with_integrity();
    let err = state.append(event(EventType::TaskCompleted, "task-1", "task"));
    assert!(err.is_err(), "orphan TaskCompleted must be rejected");
}

#[test]
fn with_integrity_task_lifecycle_passes() {
    let state = make_state().with_integrity();
    state
        .append(event(EventType::TaskCreated, "task-1", "task"))
        .unwrap();
    state
        .append(event(EventType::TaskStarted, "task-1", "task"))
        .unwrap();
    state
        .append(event(EventType::TaskCompleted, "task-1", "task"))
        .unwrap();
}
