//! Tests for first-class semantic state objects (SEMANTIC_EVENTS §8): Risk
//! (full lifecycle), Assumption + Constraint (record-only notes), and their
//! surfacing in the derived Project Plan.
//!
//! Creation of these objects may need the PM/LLM to *interpret* an observation,
//! but their state transitions are deterministic reducers — this is the
//! "agents interpret, the system records" boundary.

use casting::actions::{validate, PmAction, PolicyError};
use casting::cursor::CursorStore;
use casting::event::{Actor, Event, EventType};
use casting::pm::AppState;
use casting::projection::{Projection, RiskStatus};
use casting::sqlite_store::SqliteEventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = CursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-sem")
}

fn cause_msg(seq: &str) -> Event {
    Event::new(
        "proj-sem",
        Actor::Agent { id: "pm".into() },
        EventType::MessageSent,
        casting::event::Aggregate {
            kind: "message".into(),
            id: format!("msg-{seq}"),
        },
        serde_json::json!({ "to": "pm", "body": "watch out" }),
    )
}

#[test]
fn risk_raises_and_resolves_via_the_gate() {
    let state = make_state();
    let cause = cause_msg("1");

    {
        let evs = (PmAction::RaiseRisk {
            id: "risk-1".into(),
            subject: "Data loss during migration".into(),
            severity: "high".into(),
        })
        .to_events("proj-sem", "pm", &cause, "corr-1");
        assert_eq!(evs[0].event_type, EventType::RiskRaised);
        for e in evs {
            state.append(e).unwrap();
        }
    }

    let proj = Projection::build(&state.store, "proj-sem").unwrap();
    let risk = proj.risks.iter().find(|r| r.id == "risk-1").unwrap();
    assert_eq!(risk.subject, "Data loss during migration");
    assert_eq!(risk.severity, "high");
    assert_eq!(risk.status, RiskStatus::Open);
    assert_eq!(risk.discovered_by, "pm");

    // Resolve it through the gate (must exist).
    let ok = validate(
        &(PmAction::ResolveRisk {
            risk_id: "risk-1".into(),
            status: RiskStatus::Resolved,
        }),
        "pm",
        &proj,
    );
    assert!(ok.is_ok());

    let evs = (PmAction::ResolveRisk {
        risk_id: "risk-1".into(),
        status: RiskStatus::Resolved,
    })
    .to_events("proj-sem", "pm", &cause, "corr-2");
    for e in evs {
        state.append(e).unwrap();
    }
    let proj = Projection::build(&state.store, "proj-sem").unwrap();
    let risk = proj.risks.iter().find(|r| r.id == "risk-1").unwrap();
    assert_eq!(risk.status, RiskStatus::Resolved);
}

#[test]
fn cannot_resolve_a_risk_that_does_not_exist() {
    let state = make_state();
    let proj = Projection::build(&state.store, "proj-sem").unwrap();
    let err = validate(
        &(PmAction::ResolveRisk {
            risk_id: "risk-nope".into(),
            status: RiskStatus::Resolved,
        }),
        "pm",
        &proj,
    )
    .expect_err("resolving a missing risk must be rejected");
    assert!(matches!(err, PolicyError::RiskNotFound(_)));
}

#[test]
fn assumptions_and_constraints_are_recorded_semantic_notes() {
    let state = make_state();
    let cause = cause_msg("2");

    for ev in (PmAction::RecordAssumption {
        id: "assume-1".into(),
        body: "Users can reach the internet".into(),
    })
    .to_events("proj-sem", "pm", &cause, "corr-1")
    {
        state.append(ev).unwrap();
    }
    for ev in (PmAction::RecordConstraint {
        id: "constr-1".into(),
        body: "Must run in a single binary".into(),
    })
    .to_events("proj-sem", "pm", &cause, "corr-1")
    {
        state.append(ev).unwrap();
    }

    let proj = Projection::build(&state.store, "proj-sem").unwrap();
    assert_eq!(proj.assumptions[0].body, "Users can reach the internet");
    assert_eq!(proj.assumptions[0].recorded_by, "pm");
    assert_eq!(proj.constraints[0].body, "Must run in a single binary");
    assert_eq!(proj.constraints[0].recorded_by, "pm");
}

#[test]
fn open_risks_surface_in_the_plan_and_resolved_ones_do_not() {
    let state = make_state();
    let cause = cause_msg("3");

    for ev in (PmAction::RaiseRisk {
        id: "risk-open".into(),
        subject: "Migration data loss".into(),
        severity: "high".into(),
    })
    .to_events("proj-sem", "pm", &cause, "corr-1")
    {
        state.append(ev).unwrap();
    }
    for ev in (PmAction::RaiseRisk {
        id: "risk-closed".into(),
        subject: "Transient network blip".into(),
        severity: "low".into(),
    })
    .to_events("proj-sem", "pm", &cause, "corr-1")
    {
        state.append(ev).unwrap();
    }
    for ev in (PmAction::ResolveRisk {
        risk_id: "risk-closed".into(),
        status: RiskStatus::Resolved,
    })
    .to_events("proj-sem", "pm", &cause, "corr-2")
    {
        state.append(ev).unwrap();
    }

    let proj = Projection::build(&state.store, "proj-sem").unwrap();
    assert_eq!(
        proj.plan().open_risks,
        vec!["Migration data loss".to_string()]
    );
}
