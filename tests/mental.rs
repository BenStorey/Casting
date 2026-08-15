//! Tests for the operating picture (`Projection::operating_model()` + GET
//! /api/model) — "what the models are seeing": priorities, governance, recorded
//! knowledge, and the per-actor contexts each model is handed.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

fn state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-model")
}

fn append(state: &AppState, etype: EventType, id: &str, kind: &str, data: serde_json::Value) {
    state
        .append(Event::new(
            &state.project,
            Actor::Owner,
            etype,
            Aggregate {
                kind: kind.into(),
                id: id.into(),
            },
            data,
        ))
        .unwrap();
}

#[test]
fn operating_model_surfaces_priorities_governance_knowledge_and_context() {
    let st = state();
    append(
        &st,
        EventType::ProjectCreated,
        "p",
        "project",
        serde_json::json!({}),
    );
    // Two tasks at different priorities.
    append(
        &st,
        EventType::TaskCreated,
        "task-1",
        "task",
        serde_json::json!({ "title": "Build a todo app", "priority": "critical" }),
    );
    append(
        &st,
        EventType::TaskCreated,
        "task-2",
        "task",
        serde_json::json!({ "title": "Add CI", "priority": "low" }),
    );
    // An active directive (governance).
    append(
        &st,
        EventType::ProjectDirectiveCreated,
        "dir-1",
        "directive",
        serde_json::json!({
            "kind": "policy",
            "statement": "Ensure write-time integrity",
            "scope": ["engineering"],
            "strength": "required",
            "created_by": "owner",
        }),
    );
    // An assumption + an opinion + a fact (knowledge).
    append(
        &st,
        EventType::AssumptionRecorded,
        "as-1",
        "assumption",
        serde_json::json!({ "body": "We assume the store sustains 1k writes/sec" }),
    );
    append(
        &st,
        EventType::OpinionRecorded,
        "op-1",
        "opinion",
        serde_json::json!({
            "subject": "databases",
            "category": "rationale",
            "statement": "Postgres is a good default for our event log",
        }),
    );
    append(
        &st,
        EventType::FactRecorded,
        "f-1",
        "fact",
        serde_json::json!({ "kind": "loc", "statement": "the repo is 1,342 lines" }),
    );

    let proj = Projection::build(&st.store, &st.project).unwrap();
    let m = proj.operating_model();

    // Priorities: critical task-1 first, low task-2 second.
    assert_eq!(m.priorities[0].task_id, "task-1");
    assert_eq!(m.priorities[1].task_id, "task-2");

    // Governance: the active directive is surfaced.
    assert_eq!(m.governance.active_directives.len(), 1);
    assert!(m.governance.active_directives[0].contains("Ensure write-time integrity"));

    // Knowledge: assumption + active opinion + fact all present.
    assert!(m
        .knowledge
        .assumptions
        .iter()
        .any(|a| a.contains("1k writes/sec")));
    assert!(m
        .knowledge
        .opinions
        .iter()
        .any(|o| o.contains("Postgres is a good default")));
    assert!(m.knowledge.facts.iter().any(|f| f.contains("1,342 lines")));

    // Context counts.
    assert_eq!(m.context.task_counts.total, 2);
    assert_eq!(m.context.task_counts.open, 2);

    // Per-actor contexts include the owner (objective + priorities visible).
    assert!(m.actor_contexts.iter().any(|c| c.actor == "owner"));
    let owner = m
        .actor_contexts
        .iter()
        .find(|c| c.actor == "owner")
        .unwrap();
    assert_eq!(owner.priorities.len(), 2);
}

#[test]
fn operating_model_reports_mechanical_drift() {
    let st = state();
    append(
        &st,
        EventType::ProjectCreated,
        "p",
        "project",
        serde_json::json!({}),
    );
    // Two ACTIVE opinions on the SAME subject (a contradiction the reconciler
    // would clean up) -> the operating picture flags it as a drift signal.
    append(
        &st,
        EventType::OpinionRecorded,
        "op-a",
        "opinion",
        serde_json::json!({
            "subject": "databases",
            "category": "design",
            "statement": "SQLite first",
        }),
    );
    append(
        &st,
        EventType::OpinionRecorded,
        "op-b",
        "opinion",
        serde_json::json!({
            "subject": "databases",
            "category": "design",
            "statement": "Actually Postgres first",
        }),
    );

    let proj = Projection::build(&st.store, &st.project).unwrap();
    let m = proj.operating_model();
    assert!(
        !m.drift_signals.is_empty(),
        "same-subject double-Active should surface as a drift signal"
    );
}

#[test]
fn operating_model_lists_superseded_opinions_in_audit() {
    let st = state();
    append(
        &st,
        EventType::ProjectCreated,
        "p",
        "project",
        serde_json::json!({}),
    );
    append(
        &st,
        EventType::OpinionRecorded,
        "op-a",
        "opinion",
        serde_json::json!({
            "subject": "databases",
            "category": "design",
            "statement": "old view",
        }),
    );
    append(
        &st,
        EventType::OpinionRecorded,
        "op-b",
        "opinion",
        serde_json::json!({
            "subject": "databases",
            "category": "design",
            "statement": "new view",
        }),
    );
    append(
        &st,
        EventType::OpinionSuperseded,
        "op-a",
        "opinion",
        serde_json::json!({ "superseded_by": "op-b" }),
    );

    let proj = Projection::build(&st.store, &st.project).unwrap();
    let m = proj.operating_model();
    // op-a in the audit trail, op-b is the current opinion.
    assert!(m
        .knowledge
        .superseded_opinions
        .iter()
        .any(|s| s.starts_with("databases: old view")));
    assert!(m.knowledge.opinions.iter().any(|o| o.contains("new view")));
}
