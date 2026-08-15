//! Cross-cutting regression guard for the to_events ↔ projection.apply seam.
//!
//! The project's whole axiom is "events are mutations, projections are state."
//! The projection reducer reads `event.data` back by string key
//! (`string_field(...)`, `unwrap_or_default()`), so a field-name or shape
//! mismatch between what `actions::to_events` WRITES and what
//! `Projection::apply` READS compiles cleanly and silently degrades. This file
//! is the cheap guard that pins the two together:
//!
//!   1. Equivalence: folding the log incrementally (apply, one event at a
//!      time — what the PM loop does) MUST equal a full fold from scratch
//!      (`Projection::build`). If the two modes ever diverge, the PM's running
//!      projection is not the same truth as a fresh fold → drift.
//!   2. Round-trip: for drift-prone newer actions, drive
//!      `validate → to_events → apply` and assert the exact derived state.

use casting::actions::{validate, PmAction};
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::plan::Priority;
use casting::pm::AppState;
use casting::projection::{DecisionStatus, OpinionStatus, Projection};
use casting::store::EventStore;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

const P: &str = "proj-rt";

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, P)
}

fn raw(
    state: &AppState,
    actor: &str,
    id: &str,
    kind: &str,
    ty: EventType,
    data: serde_json::Value,
) {
    state
        .append(Event::new(
            P,
            Actor::Agent { id: actor.into() },
            ty,
            Aggregate {
                kind: kind.into(),
                id: id.into(),
            },
            data,
        ))
        .unwrap();
}

fn cause() -> Event {
    Event::new(
        P,
        Actor::Owner,
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "msg-cause".into(),
        },
        serde_json::json!({ "to": "pm", "body": "x" }),
    )
}

/// Set up a small but multi-faceted log: a task, two decisions, two
/// same-category opinions — then run the drift-prone actions through the gate.
fn seed_run_actions(state: &AppState) {
    raw(
        state,
        "pm",
        "task-1",
        "task",
        EventType::TaskCreated,
        serde_json::json!({ "title": "Build thing", "kind": "feature" }),
    );
    raw(
        state,
        "pm",
        "d-v1",
        "decision",
        EventType::DecisionProposed,
        serde_json::json!({
            "subject": "SQLite", "options": {}, "recommendation": "A",
            "class": "database", "involvement": "ask",
        }),
    );
    raw(
        state,
        "pm",
        "d-v2",
        "decision",
        EventType::DecisionProposed,
        serde_json::json!({
            "subject": "Postgres", "options": {}, "recommendation": "B",
            "class": "database", "involvement": "ask",
        }),
    );
    raw(
        state,
        "pm",
        "op-1",
        "opinion",
        EventType::OpinionRecorded,
        serde_json::json!({ "category": "design", "statement": "First take" }),
    );
    raw(
        state,
        "pm",
        "op-2",
        "opinion",
        EventType::OpinionRecorded,
        serde_json::json!({ "category": "design", "statement": "Second take" }),
    );

    // --- Drive the drift-prone actions through the full pipeline ---
    // Set a priority (TaskPriorityChanged to reducer field).
    let proj = Projection::build(&state.store, P).unwrap();
    let set_pri = PmAction::SetTaskPriority {
        task_id: "task-1".into(),
        priority: Priority::High,
    };
    validate(&set_pri, "pm", &proj).unwrap();
    for e in set_pri.to_events(P, "pm", &cause(), "corr") {
        state.append(e).unwrap();
    }

    // Supersede an opinion (OpinionSuperseded flips status).
    let proj = Projection::build(&state.store, P).unwrap();
    let sup_op = PmAction::SupersedeOpinion {
        opinion_id: "op-1".into(),
        by_opinion_id: "op-2".into(),
    };
    validate(&sup_op, "pm", &proj).unwrap();
    for e in sup_op.to_events(P, "pm", &cause(), "corr") {
        state.append(e).unwrap();
    }

    // Supersede a decision (DecisionSuperseded sets Superseded + link).
    let proj = Projection::build(&state.store, P).unwrap();
    let sup_dec = PmAction::SupersedeDecision {
        decision_id: "d-v1".into(),
        by_decision_id: "d-v2".into(),
    };
    validate(&sup_dec, "pm", &proj).unwrap();
    for e in sup_dec.to_events(P, "pm", &cause(), "corr") {
        state.append(e).unwrap();
    }
}

#[test]
fn incremental_apply_equals_full_fold() {
    let state = make_state();
    seed_run_actions(&state);

    // All events, in append order (the single source of truth).
    let all = state.store.read_since(P, 0).unwrap();
    assert!(!all.is_empty());

    // Full fold from scratch (what a cold read does).
    let folded = Projection::build(&state.store, P).unwrap();

    // The PM's *running* projection: same construction as `build` (default +
    // project_id + one `apply` per event + a fresh derived plan), but built by
    // hand from the raw log. MUST agree with the full fold — otherwise the two
    // projection modes drift.
    let mut incremental = Projection {
        project_id: P.into(),
        ..Projection::default()
    };
    for e in &all {
        incremental.apply(e);
    }
    incremental.plan = incremental.plan();

    assert_eq!(
        incremental, folded,
        "incremental apply diverged from full fold"
    );
}

#[test]
fn roundtrip_priority_opinion_decision_derived_state() {
    let state = make_state();
    seed_run_actions(&state);
    let proj = Projection::build(&state.store, P).unwrap();

    // SetTaskPriority -> TaskPriorityChanged -> Task.priority
    let task = proj.tasks.iter().find(|t| t.id == "task-1").unwrap();
    assert_eq!(task.priority, Priority::High);

    // SupersedeOpinion -> OpinionSuperseded -> op-1 no longer active
    let op1 = proj.opinions.iter().find(|o| o.id == "op-1").unwrap();
    assert_eq!(op1.status, OpinionStatus::Superseded);
    assert!(
        proj.active_opinions().iter().all(|o| o.id != "op-1"),
        "superseded opinion leaked into active set"
    );
    assert!(proj.active_opinions().iter().any(|o| o.id == "op-2"));

    // SupersedeDecision -> DecisionSuperseded -> Superseded + link to replacement
    let d1 = proj.decisions.iter().find(|d| d.id == "d-v1").unwrap();
    assert_eq!(d1.status, DecisionStatus::Superseded);
    assert_eq!(d1.superseded_by.as_deref(), Some("d-v2"));
}

#[test]
fn fold_twice_is_stable() {
    // Folding the same log twice yields an identical projection (the reducer is
    // deterministic — no ordering/`now()`/random input in the fold).
    let state = make_state();
    seed_run_actions(&state);
    let a = Projection::build(&state.store, P).unwrap();
    let b = Projection::build(&state.store, P).unwrap();
    assert_eq!(a, b);
}
