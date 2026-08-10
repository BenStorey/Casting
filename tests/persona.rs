//! Tests for Persona / CV rendering (brief §2.2).
//!
//! A pure renderer turning an agent's derived state into a friendly identity
//! card — the persona layer sits ON TOP of real state; never authoritative.

use casting::cursor::CursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::sqlite_store::SqliteEventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = CursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-pers")
}

fn hire(state: &AppState, id: &str, role: &str) {
    state
        .append(Event::new(
            "proj-pers",
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: id.into(),
            },
            serde_json::json!({ "role": role }),
        ))
        .unwrap();
}

fn task(state: &AppState, id: &str, title: &str, kind: &str, assignee: &str) {
    state
        .append(Event::new(
            "proj-pers",
            Actor::Agent { id: "pm".into() },
            EventType::TaskCreated,
            Aggregate {
                kind: "task".into(),
                id: id.into(),
            },
            serde_json::json!({ "title": title, "kind": kind }),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-pers",
            Actor::Agent { id: "pm".into() },
            EventType::TaskAssigned,
            Aggregate {
                kind: "task".into(),
                id: id.into(),
            },
            serde_json::json!({ "assignee": assignee }),
        ))
        .unwrap();
}

fn complete(state: &AppState, id: &str) {
    state
        .append(Event::new(
            "proj-pers",
            Actor::Agent {
                id: "marcus-reed".into(),
            },
            EventType::TaskCompleted,
            Aggregate {
                kind: "task".into(),
                id: id.into(),
            },
            serde_json::json!({ "result": "done" }),
        ))
        .unwrap();
}

fn owner_directive(state: &AppState, id: &str, statement: &str, scope: &[&str]) {
    state
        .append(Event::new(
            "proj-pers",
            Actor::Owner,
            EventType::ProjectDirectiveCreated,
            Aggregate {
                kind: "directive".into(),
                id: id.into(),
            },
            serde_json::json!({
                "kind": "policy",
                "statement": statement,
                "scope": scope,
                "strength": "required",
                "created_by": "owner",
            }),
        ))
        .unwrap();
}

#[test]
fn persona_reflects_current_and_completed_work() {
    let state = make_state();
    hire(&state, "marcus-reed", "Principal Engineer");
    task(&state, "task-a", "Auth", "engineering", "marcus-reed");
    task(&state, "task-b", "Billing", "engineering", "marcus-reed");
    complete(&state, "task-a");
    owner_directive(&state, "d-tdd", "TDD required", &["engineering"]);

    let proj = Projection::build(&state.store, "proj-pers").unwrap();
    let persona = proj.persona_for("marcus-reed").unwrap();

    assert_eq!(persona.id, "marcus-reed");
    assert_eq!(persona.role, "Principal Engineer");
    assert_eq!(persona.status, "active");
    // task-a is done (counted, highlighted), task-b is current.
    assert_eq!(persona.completed_tasks, 1);
    assert_eq!(persona.highlights, vec!["Auth".to_string()]);
    assert!(persona.current_tasks.contains(&"task-b".to_string()));
    assert!(!persona.current_tasks.contains(&"task-a".to_string()));
    // Engineering directive applies.
    assert!(persona
        .directives_applicable
        .iter()
        .any(|s| s.contains("TDD")));
}

#[test]
fn persona_returns_none_for_unknown_agent() {
    let state = make_state();
    hire(&state, "marcus-reed", "Principal Engineer");
    let proj = Projection::build(&state.store, "proj-pers").unwrap();
    assert!(proj.persona_for("nobody").is_none());
}

#[test]
fn persona_serializes() {
    let state = make_state();
    hire(&state, "marcus-reed", "Principal Engineer");
    task(&state, "task-a", "Auth", "engineering", "marcus-reed");

    let proj = Projection::build(&state.store, "proj-pers").unwrap();
    let persona = proj.persona_for("marcus-reed").unwrap();
    let json = serde_json::to_string(&persona).unwrap();
    assert!(json.contains("marcus-reed"));
    assert!(json.contains("Principal Engineer"));
}
