//! Tests for `cast log` — event-stream dump + integrity verify (roadmap item 4).
//!
//! The event log is the authoritative history; dump surfaces it and verify
//! checks stream invariants (contiguous sequences, DecisionMade after
//! DecisionProposed, TaskCompleted after TaskCreated).

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::replay;
use casting::sqlite_store::SqliteEventStore;
use casting::store::EventStore;

fn make_store() -> SqliteEventStore {
    SqliteEventStore::in_memory().unwrap()
}

fn append(store: &SqliteEventStore, et: EventType, kind: &str, id: &str, data: serde_json::Value) {
    store
        .append(Event::new(
            "proj",
            Actor::Agent { id: "pm".into() },
            et,
            Aggregate {
                kind: kind.into(),
                id: id.into(),
            },
            data,
        ))
        .unwrap();
}

#[test]
fn dump_prints_one_line_per_event_in_sequence() {
    let store = make_store();
    append(
        &store,
        EventType::ProjectCreated,
        "project",
        "proj",
        serde_json::json!({}),
    );
    append(
        &store,
        EventType::TaskCreated,
        "task",
        "task-1",
        serde_json::json!({ "title": "A" }),
    );
    append(
        &store,
        EventType::TaskCompleted,
        "task",
        "task-1",
        serde_json::json!({}),
    );

    let lines = replay::dump(&store, "proj").unwrap();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("ProjectCreated"));
    assert!(lines[1].contains("task-1"));
    assert!(lines[2].contains("TaskCompleted"));
    // Sequence numbers ascend.
    assert!(lines[0].starts_with("#   1"));
    assert!(lines[2].starts_with("#   3"));
}

#[test]
fn verify_on_a_clean_stream_reports_no_problems() {
    let store = make_store();
    append(
        &store,
        EventType::TaskCreated,
        "task",
        "task-1",
        serde_json::json!({}),
    );
    append(
        &store,
        EventType::TaskCompleted,
        "task",
        "task-1",
        serde_json::json!({}),
    );
    append(
        &store,
        EventType::DecisionProposed,
        "decision",
        "d-1",
        serde_json::json!({ "subject": "db" }),
    );
    append(
        &store,
        EventType::DecisionMade,
        "decision",
        "d-1",
        serde_json::json!({ "approved": true }),
    );

    assert!(replay::verify(&store, "proj").unwrap().is_empty());
}

#[test]
fn verify_detects_an_orphan_decision_made() {
    let store = make_store();
    append(
        &store,
        EventType::TaskCreated,
        "task",
        "task-1",
        serde_json::json!({}),
    );
    // A DecisionMade with no prior DecisionProposed.
    append(
        &store,
        EventType::DecisionMade,
        "decision",
        "d-orphan",
        serde_json::json!({}),
    );

    let problems = replay::verify(&store, "proj").unwrap();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("DecisionMade without a prior DecisionProposed")),
        "problems: {problems:?}"
    );
}

#[test]
fn verify_detects_a_task_completed_without_created() {
    let store = make_store();
    append(
        &store,
        EventType::TaskCompleted,
        "task",
        "task-nope",
        serde_json::json!({}),
    );
    let problems = replay::verify(&store, "proj").unwrap();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("TaskCompleted without a prior TaskCreated")),
        "problems: {problems:?}"
    );
}
