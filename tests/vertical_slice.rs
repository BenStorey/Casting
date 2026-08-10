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

    // Find the Database decision specifically (it is Ask-class -> owner decides).
    let proj = Projection::build(&state.store, "proj-test").unwrap();
    let decision_id = proj
        .decisions
        .iter()
        .find(|d| d.subject == "Database choice")
        .map(|d| d.id.clone())
        .expect("the Database decision should be proposed to the owner");
    let subject = proj
        .decisions
        .iter()
        .find(|d| d.subject == "Database choice")
        .unwrap()
        .subject
        .clone();
    state
        .append(Event::new(
            "proj-test",
            Actor::Owner,
            EventType::DecisionMade,
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
    // Ask-class decision: the OWNER decided it.
    assert_eq!(dec.decided_by.as_deref(), Some("owner"));
    // PM acknowledged (declined path), so there's a reply to the owner.
    assert!(
        proj.messages.iter().any(|m| m.to == "owner"),
        "PM should acknowledge the decision"
    );
}

#[tokio::test]
async fn ask_class_decision_stays_in_owner_inbox_with_policy_typed() {
    let state = make_state();
    state.append(owner_message("Build a thing")).unwrap();
    casting::pm::drive_pm(&state).await.unwrap();
    let proj = Projection::build(&state.store, "proj-test").unwrap();

    let db = proj
        .decisions
        .iter()
        .find(|d| d.subject == "Database choice")
        .expect("Database decision exists");
    // Database is an Ask-class decision: typed as such, unresolved (Proposed).
    assert_eq!(db.status, DecisionStatus::Proposed);
    assert_eq!(db.class, casting::policy::DecisionClass::Database);
    assert_eq!(db.involvement, casting::policy::OwnerInvolvement::Ask);
    assert_eq!(db.decided_by, None);
}

#[tokio::test]
async fn pm_class_decision_is_delegated_via_universal_pair() {
    let state = make_state();
    state.append(owner_message("Build a thing")).unwrap();
    casting::pm::drive_pm(&state).await.unwrap();
    let proj = Projection::build(&state.store, "proj-test").unwrap();

    // TestingLibrary is a Pm-class decision: the PM decides it itself via the
    // SAME event pair (DecisionProposed -> DecisionMade, actor = pm).
    let tl = proj
        .decisions
        .iter()
        .find(|d| d.subject == "Automated-testing library")
        .expect("TestingLibrary decision exists");
    assert_eq!(tl.class, casting::policy::DecisionClass::TestingLibrary);
    assert_eq!(tl.involvement, casting::policy::OwnerInvolvement::Pm);
    assert_eq!(tl.status, DecisionStatus::Approved);
    assert_eq!(tl.decided_by.as_deref(), Some("pm"));

    // It must NOT be in the owner's inbox (inbox = Proposed decisions only).
    let open: Vec<_> = proj
        .decisions
        .iter()
        .filter(|d| d.status == DecisionStatus::Proposed)
        .collect();
    assert!(
        !open
            .iter()
            .any(|d| d.subject == "Automated-testing library"),
        "a delegated decision should never sit in the owner inbox"
    );

    // And a follow-up task was created for it.
    assert!(
        proj.tasks.iter().any(|t| t.id == "task-testing-lib"),
        "PM should create a task for the delegated decision"
    );

    // The full universal pair is recorded in the event log as DecisionMade by
    // the PM (not the owner) — proving the event type carries the decider.
    let made_events = state
        .store
        .read_since("proj-test", 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == EventType::DecisionMade)
        .collect::<Vec<_>>();
    assert!(
        made_events
            .iter()
            .any(|e| e.actor == Actor::Agent { id: "pm".into() } && e.aggregate.id == tl.id),
        "the universal DecisionMade should be authored by the delegated PM"
    );
}
