//! Tests for the liveness watchdog — the "dead man's switch"
//! (docs/plans/2026-08-13_harness-guards.md, feature 3). Detects a cast that is
//! ALIVE but NOT MAKING PROGRESS and self-actuates a WorkPaused (the same
//! resumable pause rail as /api/pause); it does NOT notify (deferred).

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::runtime::watchdog::{self, SignalKind, WatchConfig, WatchModel};
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
use chrono::{Duration, Utc};

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj-watch");
    state
        .append(Event::new(
            "proj-watch",
            Actor::System,
            EventType::ProjectCreated,
            Aggregate {
                kind: "project".into(),
                id: "proj-watch".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    state
}

fn append_error(state: &AppState, n: usize, error: &str) {
    for i in 0..n {
        state
            .append(Event::new(
                "proj-watch",
                Actor::System,
                EventType::ActivityFailed,
                Aggregate {
                    kind: "task".into(),
                    id: "t-1".into(),
                },
                serde_json::json!({ "id": format!("t-1-{i}"), "error": error }),
            ))
            .unwrap();
    }
}

// --- Pure detect() on a derived model ---

#[test]
fn detect_finds_retry_loop() {
    let cfg = WatchConfig {
        max_repeat_errors: 3,
        ..Default::default()
    };
    let model = WatchModel {
        repeated_errors: vec![("boom x".into(), 5)],
        ..Default::default()
    };
    let sig = watchdog::detect(&cfg, &model).expect("retry loop detected");
    assert_eq!(sig.kind, SignalKind::RetryLoop);
    assert!(sig.detail.contains("5x"));
}

#[test]
fn detect_flags_no_progress_only_with_in_flight_work() {
    let cfg = WatchConfig {
        stall_hours: 24,
        ..Default::default()
    };

    // Stalled AND active -> NoProgress.
    let stalled = WatchModel {
        has_started: true,
        in_flight_work: true,
        last_event_age: Duration::hours(30),
        ..Default::default()
    };
    let sig = watchdog::detect(&cfg, &stalled).expect("no-progress detected");
    assert_eq!(sig.kind, SignalKind::NoProgress);

    // Idle board (all done / waiting on director) is NOT a stall — don't false-
    // positive a legitimately-quiet cast.
    let idle = WatchModel {
        has_started: true,
        in_flight_work: false,
        last_event_age: Duration::hours(30),
        ..Default::default()
    };
    assert!(watchdog::detect(&cfg, &idle).is_none());

    // Active but recent events -> not yet stalled.
    let fresh = WatchModel {
        has_started: true,
        in_flight_work: true,
        last_event_age: Duration::hours(1),
        ..Default::default()
    };
    assert!(watchdog::detect(&cfg, &fresh).is_none());
}

// --- scan() derives repeated errors from the log ---

#[test]
fn scan_derives_repeated_errors_from_log() {
    let state = make_state();
    append_error(&state, 4, "boom repeated");
    let model = watchdog::scan(&state, Utc::now());
    let (err, n) = model
        .repeated_errors
        .iter()
        .find(|(e, _)| e == "boom repeated")
        .expect("error counted");
    assert_eq!((err.as_str(), *n), ("boom repeated", 4));
}

// --- audit() self-actuates (retry-loop path, no task events needed) ---

#[test]
fn audit_auto_pauses_on_retry_loop() {
    let state = make_state();
    append_error(&state, 5, "boom repeated");

    let cfg = WatchConfig {
        max_repeat_errors: 3,
        ..Default::default()
    };
    let reason = watchdog::audit(&state, &cfg, Utc::now())
        .unwrap()
        .expect("paused");

    assert!(reason.contains("repeated"));
    // The cast is now paused (WorkPaused folded in), by the system watchdog.
    let proj = state.projection().unwrap();
    let p = proj.paused.clone().expect("paused");
    assert_eq!(p.by, "system");
    assert!(casting::pm::guard::is_paused(&proj));
    // ... and the dispatch gate now blocks work.
    assert!(casting::pm::guard::llm_dispatch_allowed(&proj).is_err());
}

#[test]
fn audit_is_idempotent_and_owner_can_resume() {
    let state = make_state();
    append_error(&state, 5, "boom again");
    let cfg = WatchConfig {
        max_repeat_errors: 3,
        ..Default::default()
    };

    assert!(watchdog::audit(&state, &cfg, Utc::now()).unwrap().is_some());

    // Second pass: already paused -> no re-pause, no duplicate event.
    assert!(watchdog::audit(&state, &cfg, Utc::now()).unwrap().is_none());
    let count = state
        .store
        .read_since("proj-watch", 0)
        .unwrap()
        .iter()
        .filter(|e| e.event_type == EventType::WorkPaused)
        .count();
    assert_eq!(count, 1, "only one WorkPaused event");

    // Owner resumes; a subsequent stall would re-pause (the model resets).
    for ev in (casting::actions::PmAction::ResumeWork).to_events(
        "proj-watch",
        "owner",
        &state
            .store
            .read_since("proj-watch", 0)
            .unwrap()
            .pop()
            .unwrap(),
        "resume",
        None,
    ) {
        state.append(ev).unwrap();
    }
    assert!(state.projection().unwrap().paused.is_none());
}

#[test]
fn no_signal_does_not_pause() {
    let state = make_state();
    // Only a ProjectCreated event, no errors, nothing in flight.
    append_error(&state, 1, "one-off"); // under repeat threshold
    let cfg = WatchConfig::default();
    assert!(watchdog::audit(&state, &cfg, Utc::now()).unwrap().is_none());
    assert!(state.projection().unwrap().paused.is_none());
}
