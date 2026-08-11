//! Tests for the decision lifecycle maturity — supersession (roadmap item 3).
//!
//! SEMANTIC_EVENTS §22: a decision can be superseded by a newer one; the old is
//! never deleted — status becomes Superseded and `superseded_by` links the
//! replacement. History is preserved.

use casting::actions::{validate, PmAction, PolicyError};
use casting::cursor::SqliteCursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::{DecisionStatus, Projection};
use casting::sqlite_store::SqliteEventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-dec")
}

fn append_proposal(state: &AppState, id: &str, subject: &str) {
    state
        .append(Event::new(
            "proj-dec",
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
fn supersede_marks_old_decision_superseded_and_links_replacement() {
    let state = make_state();
    append_proposal(&state, "d-v1", "SQLite");
    append_proposal(&state, "d-v2", "Postgres");

    // Supersede d-v1 with d-v2 (history preserved).
    let cause = Event::new(
        "proj-dec",
        Actor::Agent { id: "pm".into() },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "msg-1".into(),
        },
        serde_json::json!({ "to": "pm", "body": "x" }),
    );
    let evs = PmAction::SupersedeDecision {
        decision_id: "d-v1".into(),
        by_decision_id: "d-v2".into(),
    }
    .to_events("proj-dec", "pm", &cause, "corr-1");
    assert_eq!(evs[0].event_type, EventType::DecisionSuperseded);
    for e in evs {
        state.append(e).unwrap();
    }

    let proj = Projection::build(&state.store, "proj-dec").unwrap();
    // Both are preserved; the old is Superseded, the new stays Proposed.
    assert_eq!(proj.decisions.len(), 2);
    let v1 = proj.decisions.iter().find(|d| d.id == "d-v1").unwrap();
    assert_eq!(v1.status, DecisionStatus::Superseded);
    assert_eq!(v1.superseded_by.as_deref(), Some("d-v2"));
    let v2 = proj.decisions.iter().find(|d| d.id == "d-v2").unwrap();
    assert_eq!(v2.status, DecisionStatus::Proposed);
}

#[test]
fn gate_requires_both_decisions_to_exist_to_supersede() {
    let state = make_state();
    append_proposal(&state, "d-v1", "SQLite");
    let proj = Projection::build(&state.store, "proj-dec").unwrap();

    // Supersede onto a nonexistent replacement is rejected.
    let err = validate(
        &PmAction::SupersedeDecision {
            decision_id: "d-v1".into(),
            by_decision_id: "d-missing".into(),
        },
        "pm",
        &proj,
    )
    .expect_err("superseding onto a missing decision must be rejected");
    assert!(matches!(err, PolicyError::DecisionNotFound(_)));

    // Superseding a nonexistent decision is rejected.
    let err = validate(
        &PmAction::SupersedeDecision {
            decision_id: "d-missing".into(),
            by_decision_id: "d-v1".into(),
        },
        "pm",
        &proj,
    )
    .expect_err("superseding a missing decision must be rejected");
    assert!(matches!(err, PolicyError::DecisionNotFound(_)));
}

#[test]
fn superseded_decision_is_not_candidate_in_plan_open_decisions() {
    let state = make_state();
    append_proposal(&state, "d-open", "Database choice");
    append_proposal(&state, "d-old", "Old idea");
    // Supersede d-old.
    let cause = Event::new(
        "proj-dec",
        Actor::Agent { id: "pm".into() },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "msg-1".into(),
        },
        serde_json::json!({ "to": "pm", "body": "x" }),
    );
    for e in (PmAction::SupersedeDecision {
        decision_id: "d-old".into(),
        by_decision_id: "d-open".into(),
    })
    .to_events("proj-dec", "pm", &cause, "corr-1")
    {
        state.append(e).unwrap();
    }

    let proj = Projection::build(&state.store, "proj-dec").unwrap();
    let plan = proj.plan();
    // Only the non-superseded open decision shows up.
    assert_eq!(plan.open_decisions, vec!["Database choice".to_string()]);
}
