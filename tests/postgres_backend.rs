//! Integration tests for the Postgres storage backend — behind the SAME traits
//! as SQLite (owner principle: swap backends freely behind the abstraction).
//!
//! Run against a real Postgres (not a mock), via the __CAST_PG_URL` env var
//! (or a default localhost config). Skipped when Postgres is unreachable so the
//! suite stays green on machines without it.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::store::CursorStore;
use casting::store::EventStore;
use casting::store::PostgresBackend;
use casting::store::SnapshotStore;

fn pg_url() -> Option<String> {
    // Allow override; default to the dev/test Postgres (docker on :55432).
    std::env::var("CAST_PG_URL").ok().or_else(|| {
        Some("host=127.0.0.1 port=55432 user=casting password=castpw dbname=casting".to_string())
    })
}

fn connect() -> Option<PostgresBackend> {
    let url = pg_url()?;
    match PostgresBackend::connect(&url) {
        Ok(b) => Some(b),
        Err(e) => {
            eprintln!("Postgres unavailable (skipping): {e}");
            None
        }
    }
}

fn sample(proj: &str, seq: i64, kind: EventType) -> Event {
    let mut e = Event::new(
        proj,
        Actor::System,
        kind,
        Aggregate {
            kind: "task".into(),
            id: format!("t-{seq}"),
        },
        serde_json::json!({ "n": seq }),
    );
    e.sequence = seq;
    e
}

#[test]
fn postgres_event_store_round_trip() {
    let Some(store) = connect() else { return };
    let proj = format!("pg-roundtrip-{}", uuid::Uuid::new_v4());

    let a = store
        .append(sample(&proj, 0, EventType::TaskCreated))
        .unwrap();
    let b = store
        .append(sample(&proj, 0, EventType::TaskStarted))
        .unwrap();
    // Sequences are assigned 1..N by the store, monotonic per project.
    assert!(
        a.sequence == 1 && b.sequence == 2,
        "got {} {}",
        a.sequence,
        b.sequence
    );
    assert_eq!(store.latest_sequence(&proj).unwrap(), 2);

    let events = store.read_since(&proj, 0).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, EventType::TaskCreated);
    assert_eq!(events[1].event_type, EventType::TaskStarted);

    // list_projects includes it.
    assert!(store.list_projects().unwrap().contains(&proj));
}

#[test]
fn postgres_cursor_round_trip() {
    let Some(store) = connect() else { return };
    let proj = format!("pg-cursor-{}", uuid::Uuid::new_v4());

    assert_eq!(store.get(&proj, "pm").unwrap().last_seen, 0);
    store.advance(&proj, "pm", 42).unwrap();
    assert_eq!(store.get(&proj, "pm").unwrap().last_seen, 42);
    store.advance(&proj, "pm", 43).unwrap();
    assert_eq!(store.get(&proj, "pm").unwrap().last_seen, 43);
}

#[test]
fn postgres_snapshot_round_trip() {
    let Some(store) = connect() else { return };
    let proj = format!("pg-snap-{}", uuid::Uuid::new_v4());

    assert!(store.load(&proj).is_none());
    // Seed a project and build a real projection, then snapshot it.
    store
        .append(sample(&proj, 0, EventType::ProjectCreated))
        .unwrap();
    let projection = casting::projection::Projection::build(&store, &proj).unwrap();
    assert_eq!(projection.project_id, proj);

    store.save(&proj, 7, &projection).unwrap();
    let (seq, loaded) = store.load(&proj).expect("snapshot present");
    assert_eq!(seq, 7);
    assert_eq!(loaded.project_id, proj);
    store.clear(&proj).unwrap();
    assert!(store.load(&proj).is_none());
}

#[test]
fn postgres_sequence_monotonic_contiguous() {
    // Atomic sequence allocation (INSERT .. ON CONFLICT .. RETURNING) hands
    // out a contiguous, strictly-monotonic 1..N per project, even across a
    // fresh backend instance (new connection, same counters).
    let Some(store) = connect() else { return };
    let proj = format!("pg-seqmono-{}", uuid::Uuid::new_v4());

    let mut seen = Vec::new();
    for _i in 0..5 {
        let ev = store
            .append(sample(&proj, 0, EventType::TaskCreated))
            .unwrap();
        seen.push(ev.sequence);
    }
    // Exactly 1..N, no gaps, no duplicates, monotonic.
    assert_eq!(seen, vec![1i64, 2, 3, 4, 5], "contiguous sequence");
    assert_eq!(store.latest_sequence(&proj).unwrap(), 5);

    // A second, independent backend (its own connection) must continue the
    // same counter rather than restarting at 1 (cross-connection safety).
    let store2 = PostgresBackend::connect(&pg_url().unwrap()).expect("second connect");
    let ev = store2
        .append(sample(&proj, 0, EventType::TaskCreated))
        .unwrap();
    assert_eq!(ev.sequence, 6, "counter shared across connections");
    assert_eq!(store.latest_sequence(&proj).unwrap(), 6);

    // A second, independent backend (its own connection) must always report
    // healthy once its initial connect succeeds and the reconnect driver is
    // looping.
    assert!(store.is_healthy(), "healthy after reconnects");
    assert!(store2.is_healthy());
}

#[test]
fn postgres_all_stores_back_the_abstraction() {
    // The whole point: a PostgresBackend implements EventStore + CursorStore +
    // SnapshotStore, so AppState can run a company entirely on Postgres.
    let Some(store) = connect() else { return };

    fn assert_event_store(_: &dyn EventStore) {}
    fn assert_cursor_store(_: &dyn CursorStore) {}
    fn assert_snapshot_store(_: &dyn SnapshotStore) {}

    assert_event_store(&store);
    assert_cursor_store(&store);
    assert_snapshot_store(&store);
}

#[test]
fn company_boots_and_onboards_entirely_on_postgres() {
    // End-to-end: an AppState running its whole lifecycle (seed -> PM loop ->
    // owner message -> onboarding) on the Postgres backend, exactly as a hosted
    // company would. This is the strongest proof the swap is seamless.
    let Some(store) = connect() else { return };
    use casting::pm::AppState;

    let project = format!("pg-company-{}", uuid::Uuid::new_v4());
    let state = AppState::new(store.clone(), store.clone(), &project);

    // Seed (what `cast run` does): project + PM.
    use casting::event::{Actor, Aggregate, Event, EventType};
    state
        .append(Event::new(
            &project,
            Actor::System,
            EventType::ProjectCreated,
            Aggregate {
                kind: "project".into(),
                id: project.clone(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    state
        .append(Event::new(
            &project,
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: "pm".into(),
            },
            serde_json::json!({ "role": "Project Manager" }),
        ))
        .unwrap();

    // Owner asks for a build -> drive the PM loop -> onboarding should produce
    // a plan (tasks, cast hires). The Postgres client uses a blocking sync
    // connect (its own runtime), so drive_pm runs on a short-lived tokio
    // runtime only.
    state
        .append(Event::new(
            &project,
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-1".into(),
            },
            serde_json::json!({ "body": "Build me a todo app" }),
        ))
        .unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(casting::pm::drive_pm(&state)).unwrap();

    let proj = casting::projection::Projection::build(&store, &project).unwrap();
    assert!(
        !proj.tasks.is_empty(),
        "onboarding created tasks on Postgres: {}",
        proj.tasks.len()
    );
    assert!(
        proj.agents.iter().any(|a| a.id == "lead-programmer"),
        "default assignable engineer (Lead Programmer) hired on Postgres"
    );
}
