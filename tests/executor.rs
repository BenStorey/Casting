//! Durable-execution tests (durability first PR, docs/plans/2026-08-13).
//!
//! These exercise the executor's idempotency guard + boot-time re-dispatch over
//! the REAL SQLite backend, including a crash/restart (open a fresh AppState
//! over the same on-disk store) to prove no work is lost or duplicated.

use casting::event::{Actor, EventType};
use casting::pm::AppState;
use casting::runtime::executor::{self, Activity, ActivityKind, ActivityResult, ActivityRunner};
use casting::store::EventStore;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A runner that records every invocation + captured results, so tests can
/// assert "exactly one physical execution".
#[derive(Default)]
struct CountingRunner {
    calls: Arc<AtomicUsize>,
    results: Arc<Mutex<Vec<String>>>,
}

impl CountingRunner {
    fn new() -> Self {
        Self::default()
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ActivityRunner for CountingRunner {
    fn run(&self, a: &Activity) -> anyhow::Result<ActivityResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.results.lock().unwrap().push(format!("ran:{}", a.id));
        Ok(ActivityResult {
            result_ref: Some(format!("artifact/{}", a.id)),
        })
    }
}

/// A runner that always fails, to test ActivityFailed recording.
struct FailingRunner;

impl ActivityRunner for FailingRunner {
    fn run(&self, _a: &Activity) -> anyhow::Result<ActivityResult> {
        anyhow::bail!("simulated executor failure")
    }
}

fn inline_activity(id: &str) -> Activity {
    Activity {
        id: id.to_string(),
        target_id: "task-7".to_string(),
        kind: ActivityKind::Inline,
    }
}

/// AppState over the given (already-constructed) store/cursors.
fn state(store: Arc<dyn EventStore>, cursors: Arc<dyn casting::store::CursorStore>) -> AppState {
    AppState::new(store, cursors, "proj")
}

/// A crash/restart pair: build a state over the SAME on-disk store file, drop
/// it (the "crash"), then rebuild a fresh state — proving the log is the only
/// durable state and process memory is disposable.
struct RestartFixture {
    _dir: tempfile::TempDir,
    db: std::path::PathBuf,
    cursors_db: std::path::PathBuf,
    calls: Arc<AtomicUsize>,
    results: Arc<Mutex<Vec<String>>>,
}

impl RestartFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self {
            db: dir.path().join("events.db"),
            cursors_db: dir.path().join("cursors.db"),
            _dir: dir,
            calls: Arc::new(AtomicUsize::new(0)),
            results: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn open(&self) -> AppState {
        let store: Arc<dyn EventStore> = Arc::new(SqliteEventStore::open(&self.db).unwrap());
        let cursors: Arc<dyn casting::store::CursorStore> =
            Arc::new(SqliteCursorStore::open(&self.cursors_db).unwrap());
        state(store, cursors)
    }

    fn runner(&self) -> CountingRunner {
        CountingRunner {
            calls: self.calls.clone(),
            results: self.results.clone(),
        }
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[test]
fn execute_runs_and_records_completed() {
    let fx = RestartFixture::new();
    let s = fx.open();
    let act = inline_activity("task-7-inline-1");

    executor::schedule(&s, Actor::System, &act).unwrap();
    let runner = fx.runner();
    let res = executor::execute(&s, &runner, Actor::System, &act).unwrap();

    assert_eq!(res.result_ref.as_deref(), Some("artifact/task-7-inline-1"));
    assert_eq!(runner.calls(), 1);
    assert!(executor::has_completed(&s, "task-7-inline-1").unwrap());
}

#[test]
fn crash_after_completion_does_not_rerun() {
    let fx = RestartFixture::new();
    // Boot 1: schedule + complete.
    {
        let s = fx.open();
        let act = inline_activity("task-7-inline-1");
        executor::schedule(&s, Actor::System, &act).unwrap();
        executor::execute(&s, &fx.runner(), Actor::System, &act).unwrap();
    }
    // Boot 2 (restart over the same store): executing the same activity must be
    // a no-op — the idempotency guard sees ActivityCompleted in the log.
    let s = fx.open();
    let act = inline_activity("task-7-inline-1");
    // Independent runner: counts ONLY what this (restarted) boot executes.
    let runner = CountingRunner::new();
    executor::execute(&s, &runner, Actor::System, &act).unwrap();
    assert_eq!(
        runner.calls(),
        0,
        "a completed activity must never be executed again"
    );
    assert!(!executor::has_failed(&s, "task-7-inline-1").unwrap());
}

#[test]
fn crash_mid_activity_redispatch_recovers() {
    let fx = RestartFixture::new();
    // Boot 1: schedule ONLY (simulate the server dying before the result lands).
    {
        let s = fx.open();
        executor::schedule(&s, Actor::System, &inline_activity("task-7-llm-call-3")).unwrap();
    }
    // Boot 2 (restart): re-dispatch the in-flight activity exactly once.
    let s = fx.open();
    let dispatched = executor::redispatch_inflight(&s, &fx.runner(), Actor::System).unwrap();
    assert_eq!(dispatched, vec!["task-7-llm-call-3"]);
    assert_eq!(fx.calls(), 1, "in-flight activity re-executed exactly once");
    assert!(executor::has_completed(&s, "task-7-llm-call-3").unwrap());
}

#[test]
fn completed_activity_is_not_redispatch() {
    let fx = RestartFixture::new();
    {
        let s = fx.open();
        let act = inline_activity("task-7-gen-2");
        executor::schedule(&s, Actor::System, &act).unwrap();
        executor::execute(&s, &fx.runner(), Actor::System, &act).unwrap();
    }
    let s = fx.open();
    let dispatched = executor::redispatch_inflight(&s, &fx.runner(), Actor::System).unwrap();
    assert!(dispatched.is_empty(), "nothing in-flight to re-dispatch");
    // Independent runner: nothing was executed during THIS (restart) boot.
    let boot2 = CountingRunner::new();
    let dispatched2 = executor::redispatch_inflight(&s, &boot2, Actor::System).unwrap();
    assert!(dispatched2.is_empty());
    assert_eq!(boot2.calls(), 0, "no re-execution after a clean completion");
}

#[test]
fn failed_activity_is_recorded_and_not_redispatch() {
    let fx = RestartFixture::new();
    {
        let s = fx.open();
        let act = inline_activity("task-7-shell-1");
        executor::schedule(&s, Actor::System, &act).unwrap();
        let err = executor::execute(&s, &FailingRunner, Actor::System, &act).unwrap_err();
        assert!(err.to_string().contains("simulated executor failure"));
        assert!(executor::has_failed(&s, "task-7-shell-1").unwrap());
    }
    // A failed activity is TERMINATED — never blindly re-dispatched (retry is a
    // PM/decision-layer concern, not machinery).
    let s = fx.open();
    let dispatched = executor::redispatch_inflight(&s, &fx.runner(), Actor::System).unwrap();
    assert!(dispatched.is_empty());
    assert_eq!(fx.calls(), 0);
}

#[test]
fn activity_events_are_durable_and_reconstructible() {
    let fx = RestartFixture::new();
    // Set a budget so the gate's Disabled check doesn't block LLM-call
    // activity execution.
    {
        let s = fx.open();
        s.append(casting::event::Event::new(
            "proj",
            casting::event::Actor::Owner,
            casting::event::EventType::BudgetSet,
            casting::event::Aggregate {
                kind: "budget".into(),
                id: "budget".into(),
            },
            serde_json::json!({ "limit_usd": 100.0, "warn_at": 0.80 }),
        ))
        .unwrap();
    }
    let act = Activity {
        id: "task-7-llm-call-3".to_string(),
        target_id: "task-7".to_string(),
        kind: ActivityKind::LlmCall {
            prompt: "why is this red?".to_string(),
        },
    };
    {
        let s = fx.open();
        executor::schedule(&s, Actor::System, &act).unwrap();
        executor::execute(&s, &fx.runner(), Actor::System, &act).unwrap();
    }
    let s = fx.open();
    let events = s.store.read_since("proj", 0).unwrap();
    let types: Vec<EventType> = events
        .iter()
        .filter(|e| e.event_type != EventType::BudgetSet) // test helper artifact
        .map(|e| e.event_type)
        .collect();
    assert_eq!(
        types,
        vec![EventType::ActivityScheduled, EventType::ActivityCompleted]
    );
    // The scheduled event carries the full activity (reconstructable for
    // re-dispatch); the completed event carries the marker + result ref.
    // Filter out the BudgetSet helper event so we index the right positions.
    let filtered: Vec<&casting::event::Event> = events
        .iter()
        .filter(|e| e.event_type != EventType::BudgetSet)
        .collect();
    let scheduled = filtered[0];
    let reconstructed: Activity =
        serde_json::from_value(scheduled.data["activity"].clone()).unwrap();
    assert_eq!(reconstructed, act);
    assert_eq!(filtered[1].data["id"], "task-7-llm-call-3");
    assert_eq!(filtered[1].data["result_ref"], "artifact/task-7-llm-call-3");
}

#[test]
fn completed_event_broadcasts_to_wake_pm_loop() {
    let fx = RestartFixture::new();
    let s = fx.open();
    let mut rx = s.subscribe();
    let act = inline_activity("task-7-inline-9");
    executor::execute(&s, &fx.runner(), Actor::System, &act).unwrap();
    // The executor must NOT call pm.evaluate; it appends ActivityCompleted and
    // lets the broadcast wake the PM loop (which drains via its durable cursor).
    let got = rx.try_recv().expect("an event was broadcast on completion");
    assert_eq!(got.event_type, EventType::ActivityCompleted);
    assert_eq!(got.data["id"], "task-7-inline-9");
}
