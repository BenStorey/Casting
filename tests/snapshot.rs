//! Tests for projection snapshots (SEMANTIC_EVENTS §18–19).
//!
//! Snapshots are a pure optimization, never a source of truth: building from a
//! snapshot + tail events must equal building from the full log, and a
//! missing/corrupt snapshot must fall back cleanly.

use casting::cursor::CursorStore;
use casting::event::{Actor, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::snapshot::{self, SnapshotStore};
use casting::sqlite_store::SqliteEventStore;
use casting::store::EventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = CursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-snap")
}

fn hire_engineer(state: &AppState, id: &str) {
    state
        .append(Event::new(
            "proj-snap",
            Actor::System,
            EventType::AgentHired,
            casting::event::Aggregate {
                kind: "agent".into(),
                id: id.into(),
            },
            serde_json::json!({ "role": "engineer" }),
        ))
        .unwrap();
}

#[test]
fn snapshot_then_tail_equals_full_fold() {
    let state = make_state();
    hire_engineer(&state, "marcus-reed");
    hire_engineer(&state, "maya-patel");

    // Full fold = ground truth.
    let full = Projection::build(&state.store, "proj-snap").unwrap();

    // Snapshot at sequence 2 (after both hires), then one more event.
    let snapshots = SnapshotStore::in_memory().unwrap();
    let seq = state.store.latest_sequence("proj-snap").unwrap();
    snapshots.save("proj-snap", seq, &full).unwrap();
    hire_engineer(&state, "james-wilson");

    // Building from snapshot should catch only the tail (james-wilson).
    let from_snap = snapshot::build_from(&state.store, &snapshots, "proj-snap").unwrap();
    assert_eq!(from_snap.agents.len(), 3);
    assert!(from_snap.agents.iter().any(|a| a.id == "james-wilson"));
    assert!(from_snap.agents.iter().any(|a| a.id == "marcus-reed"));

    // And it equals a full rebuild of the same log.
    let full_now = Projection::build(&state.store, "proj-snap").unwrap();
    assert_eq!(from_snap.agents, full_now.agents);
}

#[test]
fn build_from_falls_back_to_full_fold_without_a_snapshot() {
    let state = make_state();
    hire_engineer(&state, "marcus-reed");
    hire_engineer(&state, "maya-patel");
    let snapshots = SnapshotStore::in_memory().unwrap(); // empty

    let proj = snapshot::build_from(&state.store, &snapshots, "proj-snap").unwrap();
    assert_eq!(proj.agents.len(), 2);
    assert_eq!(
        proj.agents,
        Projection::build(&state.store, "proj-snap").unwrap().agents
    );
}

#[test]
fn corrupt_snapshot_is_discarded_and_falls_back() {
    let state = make_state();
    hire_engineer(&state, "marcus-reed");

    // A file-backed snapshot store we can corrupt from a second connection.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("snapshots.db");
    let snapshots = SnapshotStore::open(&path).unwrap();

    // Inject a corrupt row directly.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO projections (project_id, sequence, projection) VALUES (?1, ?2, ?3)
         ON CONFLICT (project_id) DO UPDATE SET projection = excluded.projection",
        rusqlite::params!["proj-snap", 1i64, "{ not valid json"],
    )
    .unwrap();

    // Corrupt snapshot -> load() returns None -> build_from falls back to a
    // full fold, identical to ground truth. (Snapshots are disposable.)
    assert!(snapshots.load("proj-snap").is_none());
    let proj = snapshot::build_from(&state.store, &snapshots, "proj-snap").unwrap();
    assert_eq!(proj.agents.len(), 1);
    assert_eq!(
        proj.agents,
        Projection::build(&state.store, "proj-snap").unwrap().agents
    );
}

#[test]
fn snapshot_round_trips_through_the_store() {
    let state = make_state();
    hire_engineer(&state, "marcus-reed");
    hire_engineer(&state, "maya-patel");
    hire_engineer(&state, "james-wilson");

    let snapshots = SnapshotStore::in_memory().unwrap();
    let full = Projection::build(&state.store, "proj-snap").unwrap();
    let seq = state.store.latest_sequence("proj-snap").unwrap();
    snapshots.save("proj-snap", seq, &full).unwrap();

    let (loaded_seq, loaded) = snapshots.load("proj-snap").unwrap();
    assert_eq!(loaded_seq, seq);
    assert_eq!(loaded.agents.len(), 3);
    // No tail events after the snapshot: build_from returns exactly the snapshot.
    let proj = snapshot::build_from(&state.store, &snapshots, "proj-snap").unwrap();
    assert_eq!(proj.agents, full.agents);
}

#[test]
fn app_state_with_snapshots_serves_a_correct_projection() {
    // End-to-end on the real read path: with snapshots enabled, the AppState
    // projection (what /api/state serves) must equal a plain full-log build.
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = CursorStore::in_memory().unwrap();
    let state = AppState::new(store.clone(), cursors, "proj-snap")
        .with_snapshots(SnapshotStore::in_memory().unwrap());

    hire_engineer(&state, "marcus-reed");
    hire_engineer(&state, "maya-patel");

    // state.projection() warms a snapshot and returns the derived projection.
    let served = state.projection().unwrap();
    assert_eq!(served.agents.len(), 2);
    // It must equal a full rebuild from the log (snapshots never change state).
    assert_eq!(
        served.agents,
        Projection::build(&store, "proj-snap").unwrap().agents
    );
}
