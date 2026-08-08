//! Integration tests for the headless core: event store + cursor.
//! These exercise the real SQLite backend end-to-end.

use casting::cursor::CursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::sqlite_store::SqliteEventStore;
use casting::store::EventStore;

fn sample_event(project: &str, seq_data_extra: i64) -> Event {
    Event::new(
        project,
        Actor::Agent { id: "pm".into() },
        EventType::TaskCreated,
        Aggregate {
            kind: "task".into(),
            id: format!("task-{seq_data_extra}"),
        },
        serde_json::json!({"seq_marker": seq_data_extra}),
    )
}

#[test]
fn append_assigns_monotonic_sequences() {
    let store = SqliteEventStore::in_memory().unwrap();
    let p = "proj-a";

    let e1 = store.append(sample_event(p, 1)).unwrap();
    let e2 = store.append(sample_event(p, 2)).unwrap();
    let e3 = store.append(sample_event(p, 3)).unwrap();

    assert_eq!(e1.sequence, 1);
    assert_eq!(e2.sequence, 2);
    assert_eq!(e3.sequence, 3);
    assert_eq!(store.latest_sequence(p).unwrap(), 3);
}

#[test]
fn sequences_are_per_project() {
    let store = SqliteEventStore::in_memory().unwrap();
    let e1 = store.append(sample_event("project-x", 1)).unwrap();
    let e2 = store.append(sample_event("project-y", 1)).unwrap();

    // Each project gets its own counter starting at 1.
    assert_eq!(e1.sequence, 1);
    assert_eq!(e2.sequence, 1);
}

#[test]
fn read_since_is_ascending_and_sliceable() {
    let store = SqliteEventStore::in_memory().unwrap();
    let p = "proj";

    for i in 1..=5 {
        store.append(sample_event(p, i)).unwrap();
    }

    let tail = store.read_since(p, 2).unwrap();
    let ids: Vec<i64> = tail.iter().map(|e| e.sequence).collect();
    assert_eq!(ids, vec![3, 4, 5]);

    // read_since(latest) = nothing new.
    let nothing = store.read_since(p, 5).unwrap();
    assert!(nothing.is_empty());
}

#[test]
fn event_fields_round_trip_through_store() {
    let store = SqliteEventStore::in_memory().unwrap();
    let original = sample_event("proj", 42);
    let stored = store.append(original).unwrap();

    // Reopen a *fresh* store on the same backing store isn't possible in-memory,
    // so we read it back and check essential fields survived serialization.
    let back = store.read_since("proj", 0).unwrap();
    assert_eq!(back.len(), 1);
    let got = &back[0];
    assert_eq!(got.event_type, EventType::TaskCreated);
    assert_eq!(got.actor, Actor::Agent { id: "pm".into() });
    assert_eq!(got.aggregate.id, "task-42");
    assert_eq!(got.data["seq_marker"], 42);
    assert_eq!(got.sequence, stored.sequence);
    // Unique event_id preserved.
    assert_eq!(got.event_id, stored.event_id);
}

#[test]
fn persisted_store_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("events.db");

    {
        let store = SqliteEventStore::open(&db_path).unwrap();
        for i in 1..=4 {
            store.append(sample_event("proj", i)).unwrap();
        }
    } // drop connection

    // Reopen and confirm the history is still there.
    let store = SqliteEventStore::open(&db_path).unwrap();
    assert_eq!(store.latest_sequence("proj").unwrap(), 4);
    assert_eq!(store.read_since("proj", 0).unwrap().len(), 4);
}

#[test]
fn cursor_starts_zero_and_advances_durably() {
    let dir = tempfile::tempdir().unwrap();
    let cursor_path = dir.path().join("cursors.db");

    {
        let cursors = CursorStore::open(&cursor_path).unwrap();
        let init = cursors.get("proj", "pm").unwrap();
        assert_eq!(init.last_seen, 0);
        cursors.advance("proj", "pm", 1842).unwrap();
    }

    // Reopen: the position persisted.
    let cursors = CursorStore::open(&cursor_path).unwrap();
    let after = cursors.get("proj", "pm").unwrap();
    assert_eq!(after.last_seen, 1842);

    // A different consumer has its own independent position.
    let other = cursors.get("proj", "agent:marcus").unwrap();
    assert_eq!(other.last_seen, 0);
}
