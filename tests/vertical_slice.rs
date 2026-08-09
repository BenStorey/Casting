//! Integration tests for the vertical slice: projections + simulated PM loop.
//! Exercises the real SQLite backend, the derived current-state projection, and
//! the scripted PM control loop (owner message -> requirements/tasks/decisions).

use casting::cursor::CursorStore;
use casting::event::{Actor, Event, EventType};
use casting::pm::{AppState, PM_CONSUMER};
use casting::projection::{DecisionStatus, Projection, TaskStatus};
use casting::sqlite_store::SqliteEventStore;
use casting::store::EventStore;
use std::time::Duration;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = CursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-test").with_step_delay(Duration::ZERO)
}

fn owner_message(body: &str) -> Event {
    Event::new(
        "proj-test",
        Actor::Owner,
        EventType::MessageSent,
        casting::event::Aggregate {
            kind: "message".into(),
            id: "msg-owner".into(),
        },
        serde_json::json!({ "to": "pm", "body": body }),
    )
}

#[test]
fn projection_derives_current_state_from_events() {
    let state = make_state();
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::AgentHired,
            casting::event::Aggregate {
                kind: "agent".into(),
                id: "marcus-reed".into(),
            },
            serde_json::json!({"role": "Principal Engineer"}),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::AgentHired,
            casting::event::Aggregate {
                kind: "agent".into(),
                id: "may-patel".into(),
            },
            serde_json::json!({"role": "QA"}),
        ))
        .unwrap();
    let task_created = state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::TaskCreated,
            casting::event::Aggregate {
                kind: "task".into(),
                id: "task-1".into(),
            },
            serde_json::json!({"title": "Auth", "kind": "feature"}),
        ))
        .unwrap();
    let _ = task_created;
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::TaskAssigned,
            casting::event::Aggregate {
                kind: "task".into(),
                id: "task-1".into(),
            },
            serde_json::json!({"assignee": "marcus-reed"}),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::TaskStarted,
            casting::event::Aggregate {
                kind: "task".into(),
                id: "task-1".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::DecisionProposed,
            casting::event::Aggregate {
                kind: "decision".into(),
                id: "decision-1".into(),
            },
            serde_json::json!({"subject": "Pick DB", "recommendation": "A"}),
        ))
        .unwrap();

    let proj = Projection::build(&state.store, "proj-test").unwrap();

    assert_eq!(proj.agents.len(), 2);
    let task = proj.tasks.iter().find(|t| t.id == "task-1").unwrap();
    assert_eq!(task.status, TaskStatus::Working);
    assert_eq!(task.assignee.as_deref(), Some("marcus-reed"));
    let dec = proj
        .decisions
        .iter()
        .find(|d| d.id == "decision-1")
        .unwrap();
    assert_eq!(dec.status, DecisionStatus::Proposed);
}

#[tokio::test]
async fn simulated_pm_onboards_company_from_owner_message() {
    let state = make_state();

    // The loop must seed nothing by itself: send one owner message, then drive.
    state.append(owner_message("Build me a todo app")).unwrap();
    let authored = casting::pm::drive_pm(&state).await.unwrap();
    assert!(
        authored >= 3,
        "PM should author a non-trivial response, got {authored}"
    );

    let proj = Projection::build(&state.store, "proj-test").unwrap();
    assert_eq!(
        proj.requirements.len(),
        1,
        "requirement created from owner intent"
    );
    assert_eq!(proj.requirements[0].title, "Build me a todo app");
    assert!(
        proj.agents.iter().any(|a| a.id == "marcus-reed"),
        "engineer hired"
    );
    assert!(proj.agents.iter().any(|a| a.id == "maya-patel"), "qa hired");
    assert!(!proj.tasks.is_empty(), "tasks created");
    // The onboarding raises a decision for the owner.
    assert!(
        proj.decisions
            .iter()
            .any(|d| d.status == DecisionStatus::Proposed),
        "a decision awaiting owner exists"
    );

    // The PM's cursor advanced past everything: driving again is a no-op.
    let again = casting::pm::drive_pm(&state).await.unwrap();
    assert_eq!(again, 0, "no duplicate work from re-drain");
    let cursor = state.cursors.get("proj-test", PM_CONSUMER).unwrap();
    assert_eq!(
        cursor.last_seen,
        state.store.latest_sequence("proj-test").unwrap()
    );
}

#[tokio::test]
async fn owner_decision_is_recorded_and_pm_reacts() {
    let state = make_state();
    state.append(owner_message("Build a thing")).unwrap();
    casting::pm::drive_pm(&state).await.unwrap();

    // Owner rules on the proposed decision (reject it).
    let proj = Projection::build(&state.store, "proj-test").unwrap();
    let decision_id = proj.decisions[0].id.clone();
    let subject = proj.decisions[0].subject.clone();
    state
        .append(Event::new(
            "proj-test",
            Actor::Owner,
            EventType::OwnerDecisionRecorded,
            casting::event::Aggregate {
                kind: "decision".into(),
                id: decision_id.clone(),
            },
            serde_json::json!({"subject": subject, "approved": false, "note": "keep it simple"}),
        ))
        .unwrap();

    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-test").unwrap();
    let dec = proj.decisions.iter().find(|d| d.id == decision_id).unwrap();
    assert_eq!(dec.status, DecisionStatus::Rejected);
    assert_eq!(dec.owner_verdict.as_deref(), Some("keep it simple"));
    // PM acknowledged (declined path), so there's a reply to the owner.
    assert!(
        proj.messages.iter().any(|m| m.to == "owner"),
        "PM should acknowledge the decision"
    );
}
