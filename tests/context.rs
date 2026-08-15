//! Tests for the Context Assembler (docs/SEMANTIC_EVENTS §21).
//!
//! Combines projection + plan + governance + risks + decisions into a targeted
//! per-agent operating context, filtered by governance scope. Pure derivation.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-ctx")
}

fn hire(state: &AppState, id: &str, role: &str) {
    state
        .append(Event::new(
            "proj-ctx",
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

fn requirement(state: &AppState, title: &str) {
    state
        .append(Event::new(
            "proj-ctx",
            Actor::Agent { id: "pm".into() },
            EventType::RequirementCreated,
            Aggregate {
                kind: "requirement".into(),
                id: format!("req-{title}"),
            },
            serde_json::json!({ "title": title, "description": "..." }),
        ))
        .unwrap();
}

fn task(state: &AppState, id: &str, title: &str, kind: &str, assignee: &str) {
    state
        .append(Event::new(
            "proj-ctx",
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
            "proj-ctx",
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
            "proj-ctx",
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
            "proj-ctx",
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

fn open_decision(state: &AppState, id: &str, subject: &str) {
    state
        .append(Event::new(
            "proj-ctx",
            Actor::Agent { id: "pm".into() },
            EventType::DecisionProposed,
            Aggregate {
                kind: "decision".into(),
                id: id.into(),
            },
            serde_json::json!({
                "subject": subject,
                "options": serde_json::json!({}),
                "recommendation": "A",
                "class": "database",
                "involvement": "ask",
            }),
        ))
        .unwrap();
}

#[test]
fn agent_context_targets_their_tasks_and_governance_scope() {
    let state = make_state();
    hire(&state, "marcus-reed", "Principal Engineer");
    hire(&state, "maya-patel", "QA Consultant");
    requirement(&state, "Build a thing");
    task(
        &state,
        "task-core",
        "Implement core",
        "engineering",
        "marcus-reed",
    );
    task(&state, "task-qa", "Write tests", "qa", "maya-patel");
    owner_directive(&state, "d-tdd", "TDD is required", &["engineering"]);
    owner_directive(&state, "d-a11y", "Be accessible", &["qa"]);
    open_decision(&state, "d-db", "Database choice");

    let proj = Projection::build(&state.store, "proj-ctx").unwrap();

    let marcus = proj.context_for("marcus-reed");
    assert_eq!(marcus.objective.as_deref(), Some("Build a thing"));
    assert_eq!(marcus.my_tasks, vec!["task-core".to_string()]);
    // Marcus sees the engineering directive (TDD), not the QA one.
    assert!(marcus.active_directives.iter().any(|s| s.contains("TDD")));
    assert!(!marcus
        .active_directives
        .iter()
        .any(|s| s.contains("accessible")));
    // Open decision is visible to agents too (it's project-wide).
    assert_eq!(marcus.open_decisions, vec!["Database choice".to_string()]);

    let maya = proj.context_for("maya-patel");
    assert_eq!(maya.my_tasks, vec!["task-qa".to_string()]);
    assert!(maya
        .active_directives
        .iter()
        .any(|s| s.contains("accessible")));
    assert!(!maya.active_directives.iter().any(|s| s.contains("TDD")));
}

#[test]
fn owner_context_sees_everything() {
    let state = make_state();
    hire(&state, "marcus-reed", "Principal Engineer");
    task(
        &state,
        "task-core",
        "Implement core",
        "engineering",
        "marcus-reed",
    );
    owner_directive(&state, "d-tdd", "TDD", &["engineering"]);

    let proj = Projection::build(&state.store, "proj-ctx").unwrap();
    let owner = proj.context_for("owner");
    // Owner encapsulates but has no "my_tasks" of their own.
    assert!(owner.active_directives.iter().any(|s| s.contains("TDD")));
    assert!(owner.my_tasks.is_empty());
}

#[test]
fn completed_tasks_are_excluded_from_my_tasks() {
    let state = make_state();
    hire(&state, "marcus-reed", "Principal Engineer");
    task(&state, "task-a", "A", "engineering", "marcus-reed");
    task(&state, "task-b", "B", "engineering", "marcus-reed");
    complete(&state, "task-a");

    let proj = Projection::build(&state.store, "proj-ctx").unwrap();
    let marcus = proj.context_for("marcus-reed");
    // Only the still-open task appears.
    assert!(!marcus.my_tasks.contains(&"task-a".to_string()));
    assert!(marcus.my_tasks.contains(&"task-b".to_string()));
}

#[test]
fn context_serializes() {
    let state = make_state();
    hire(&state, "marcus-reed", "Principal Engineer");
    task(&state, "task-a", "A", "engineering", "marcus-reed");

    let proj = Projection::build(&state.store, "proj-ctx").unwrap();
    let ctx = proj.context_for("marcus-reed");
    let json = serde_json::to_string(&ctx).unwrap();
    assert!(json.contains("marcus-reed"));
    assert!(json.contains("task-a"));
}
