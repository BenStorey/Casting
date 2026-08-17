//! Integration tests for the vertical slice: projections + simulated PM loop.
//! Exercises the real SQLite backend, the derived current-state projection, and
//! the MockOrchestrator-driven PM control loop.
//!
//! The old scripted planning functions (`plan_onboard`, etc.) were removed in
//! c244d15 — they were the demo tape. These tests seed the required state
//! directly (project, agents, requirements, tasks) and verify the current
//! MockOrchestrator behavior: ack on director message, create-task / propose-
//! decision when an objective exists, per-actor lifecycle for assigned tasks.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::{AppState, PM_CONSUMER};
use casting::projection::{DecisionStatus, Projection, TaskStatus};
use casting::runtime::orchestrator::MockOrchestrator;
use casting::store::EventStore;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
use std::sync::Arc;
use std::time::Duration;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-test")
        .with_step_delay(Duration::ZERO)
        .with_orchestrator(Arc::new(MockOrchestrator))
}

#[allow(dead_code)]
fn owner_message(body: &str) -> Event {
    Event::new(
        "proj-test",
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        casting::event::Aggregate {
            kind: "message".into(),
            id: "msg-director".into(),
        },
        serde_json::json!({ "to": "mei", "body": body }),
    )
}

/// Seed project + agents + requirement so the MockOrchestrator sees an
/// objective and can plan actions (CreateTask or ProposeDecision).
/// The cursor is advanced past these seed events so the PM only reacts
/// to the subsequent director message.
fn seed_company(state: &AppState) {
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::ProjectCreated,
            casting::event::Aggregate {
                kind: "project".into(),
                id: "proj-test".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    // Set a budget so the gate's Disabled check doesn't block orchestrator
    // dispatch. Tests using MockOrchestrator need the gate to pass.
    state
        .append(Event::new(
            "proj-test",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::BudgetSet,
            casting::event::Aggregate {
                kind: "budget".into(),
                id: "budget".into(),
            },
            serde_json::json!({ "limit_usd": 100.0, "warn_at": 0.80 }),
        ))
        .unwrap();
    for (id, role) in [("diego", "engineer"), ("tess", "testing_engineer")] {
        state
            .append(Event::new(
                "proj-test",
                Actor::System,
                EventType::AgentHired,
                casting::event::Aggregate {
                    kind: "agent".into(),
                    id: id.into(),
                },
                serde_json::json!({ "role": role }),
            ))
            .unwrap();
    }
    // A requirement gives the PM an objective (derived from its title).
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::RequirementCreated,
            casting::event::Aggregate {
                kind: "requirement".into(),
                id: "req-1".into(),
            },
            serde_json::json!({
                "title": "Build me a todo app",
                "body": "Build me a todo app",
            }),
        ))
        .unwrap();
    // Advance cursor past seed events so the PM only processes the director message.
    state
        .cursors
        .advance(
            "proj-test",
            PM_CONSUMER,
            state.store.latest_sequence("proj-test").unwrap(),
        )
        .unwrap();
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
                id: "diego".into(),
            },
            serde_json::json!({"role": "Lead Developer"}),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::AgentHired,
            casting::event::Aggregate {
                kind: "agent".into(),
                id: "tess".into(),
            },
            serde_json::json!({"role": "Testing Engineer"}),
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
            serde_json::json!({"assignee": "diego"}),
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
    assert_eq!(task.assignee.as_deref(), Some("diego"));
    let dec = proj
        .decisions
        .iter()
        .find(|d| d.id == "decision-1")
        .unwrap();
    assert_eq!(dec.status, DecisionStatus::Proposed);
}

#[tokio::test]
async fn pm_acknowledges_owner_message_and_cursor_advances() {
    let state = make_state();
    seed_company(&state);

    // Send one director decision trigger (not a MessageSent, which takes the
    // deterministic chat-interface path and bypasses the orchestrator).
    state
        .append(Event::new(
            "proj-test",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::DecisionMade,
            Aggregate {
                kind: "decision".into(),
                id: "owner-decision-1".into(),
            },
            serde_json::json!({"subject": "test", "approved": true, "note": "Build me a todo app", "body": "Build me a todo app"}),
        ))
        .unwrap();
    let authored = casting::pm::drive_pm(&state).await.unwrap();
    // The MockOrchestrator sees an existing objective and the PM role, with
    // no other priorities — it creates task-mock-1 (1 action). Plus any
    // audit/telemetry events.
    assert!(authored >= 1, "PM should author a response, got {authored}");

    let proj = Projection::build(&state.store, "proj-test").unwrap();
    // The mock created a task under the adopted decision (the task id for
    // decision triggers is "task-adopt-{decision_id}").
    assert!(
        proj.tasks
            .iter()
            .any(|t| t.id == "task-adopt-owner-decision-1"),
        "mock PM should create a task from the objective"
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
    seed_company(&state);

    // Seed a Database decision (Ask-class) directly.
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::DecisionProposed,
            casting::event::Aggregate {
                kind: "decision".into(),
                id: "dec-db".into(),
            },
            serde_json::json!({
                "subject": "Database choice",
                "options": {"A": "Postgres", "B": "SQLite"},
                "recommendation": "A",
                "class": "database",
                "involvement": "ask",
            }),
        ))
        .unwrap();
    let proj = Projection::build(&state.store, "proj-test").unwrap();
    let decision_id = proj
        .decisions
        .iter()
        .find(|d| d.subject == "Database choice")
        .map(|d| d.id.clone())
        .expect("the Database decision should be proposed");

    // Owner rejects the decision.
    state
        .append(Event::new(
            "proj-test",
            Actor::Director {
            user_id: "ceo".into(),
        },
            EventType::DecisionMade,
            casting::event::Aggregate {
                kind: "decision".into(),
                id: decision_id.clone(),
            },
            serde_json::json!({"subject": "Database choice", "approved": false, "note": "keep it simple"}),
        ))
        .unwrap();

    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-test").unwrap();
    let dec = proj.decisions.iter().find(|d| d.id == decision_id).unwrap();
    assert_eq!(dec.status, DecisionStatus::Rejected);
    assert_eq!(dec.owner_verdict.as_deref(), Some("keep it simple"));
    // Ask-class decision: the DIRECTOR decided it.
    assert_eq!(dec.decided_by.as_deref(), Some("director"));
    // PM acknowledged (declined path), so there's a reply to the director.
    assert!(
        proj.messages.iter().any(|m| m.to == "director"),
        "PM should acknowledge the decision"
    );
}

#[tokio::test]
async fn ask_class_decision_stays_in_owner_inbox_with_policy_typed() {
    let state = make_state();
    seed_company(&state);
    // Seed a Database decision (Ask-class) directly.
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::DecisionProposed,
            casting::event::Aggregate {
                kind: "decision".into(),
                id: "dec-db".into(),
            },
            serde_json::json!({
                "subject": "Database choice",
                "options": {"A": "Postgres", "B": "SQLite"},
                "recommendation": "A",
                "class": "database",
                "involvement": "ask",
            }),
        ))
        .unwrap();

    let proj = Projection::build(&state.store, "proj-test").unwrap();
    let db = proj
        .decisions
        .iter()
        .find(|d| d.subject == "Database choice")
        .expect("Database decision exists");
    // Database is an Ask-class decision: typed as such, unresolved (Proposed).
    assert_eq!(db.status, DecisionStatus::Proposed);
    assert_eq!(db.class, casting::pm::policy::DecisionClass::Database);
    assert_eq!(db.involvement, casting::pm::policy::OwnerInvolvement::Ask);
    assert_eq!(db.decided_by, None);
}

#[tokio::test]
async fn pm_class_decision_is_delegated_via_universal_pair() {
    let state = make_state();
    seed_company(&state);
    // Seed a TestingLibrary decision (Pm-class, auto-decided by PM).
    state
        .append(Event::new(
            "proj-test",
            Actor::System,
            EventType::DecisionProposed,
            casting::event::Aggregate {
                kind: "decision".into(),
                id: "dec-test-lib".into(),
            },
            serde_json::json!({
                "subject": "Automated-testing library",
                "options": {"A": "Vitest", "B": "Playwright"},
                "recommendation": "A",
                "class": "testing_library",
                "involvement": "pm",
            }),
        ))
        .unwrap();

    // The PM loop doesn't auto-decide Pm-class decisions (the old scripted
    // planner did). Instead, drive the PM and verify the decision remains as
    // proposed — the universal pair is resolved by a MakeDecision action,
    // which the MockOrchestrator does not produce.
    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-test").unwrap();
    let tl = proj
        .decisions
        .iter()
        .find(|d| d.subject == "Automated-testing library")
        .expect("TestingLibrary decision exists");
    assert_eq!(tl.class, casting::pm::policy::DecisionClass::TestingLibrary);
    assert_eq!(tl.involvement, casting::pm::policy::OwnerInvolvement::Pm);
    // Without an LLM to call MakeDecision, the decision stays proposed.
    assert_eq!(tl.status, DecisionStatus::Proposed);
    assert_eq!(tl.decided_by, None);
}
