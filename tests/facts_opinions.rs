//! Tests for recording project OPINIONS (subjective knowledge) and FACTS
//! (objective point-in-time measures) as first-class events. Owner concept
//! (2026-08-10): knowledge worth not re-deriving is *opinion*; objective
//! measures are the cases we capture as point-in-time facts.

use casting::actions::{self, PmAction};
use casting::cursor::SqliteCursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::sqlite_store::SqliteEventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-k")
}

fn append(
    state: &AppState,
    etype: EventType,
    aggregate_id: &str,
    kind: &str,
    data: serde_json::Value,
) {
    state
        .append(Event::new(
            &state.project,
            Actor::Owner,
            etype,
            Aggregate {
                kind: kind.to_string(),
                id: aggregate_id.to_string(),
            },
            data,
        ))
        .unwrap();
}

fn cause_for(state: &AppState) -> Event {
    Event::new(
        &state.project,
        Actor::Owner,
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m".into(),
        },
        serde_json::json!({}),
    )
}

#[test]
fn opinion_reduces_into_projection() {
    let state = make_state();
    append(
        &state,
        EventType::OpinionRecorded,
        "op-1",
        "opinion",
        serde_json::json!({
            "category": "rationale",
            "statement": "Postgres is a good default for our event log",
        }),
    );
    let proj = Projection::build(&state.store, &state.project).unwrap();
    assert_eq!(proj.opinions.len(), 1);
    assert_eq!(proj.opinions[0].id, "op-1");
    assert_eq!(proj.opinions[0].category, "rationale");
    assert_eq!(proj.opinions[0].recorded_by, "owner");
    assert_eq!(proj.opinions[0].supersedes, None);
}

#[test]
fn opinion_supersedes_keeps_history() {
    let state = make_state();
    append(
        &state,
        EventType::OpinionRecorded,
        "op-1",
        "opinion",
        serde_json::json!({ "category": "design", "statement": "First take" }),
    );
    append(
        &state,
        EventType::OpinionRecorded,
        "op-2",
        "opinion",
        serde_json::json!({
            "category": "design",
            "statement": "FoundationDB fits the ordered-log shape better",
            "supersedes": "op-1",
        }),
    );
    let proj = Projection::build(&state.store, &state.project).unwrap();
    assert_eq!(proj.opinions.len(), 2, "history preserved, nothing edited");
    assert_eq!(proj.opinions[1].supersedes.as_deref(), Some("op-1"));
}

#[test]
fn fact_reduces_with_point_in_time() {
    let state = make_state();
    append(
        &state,
        EventType::FactRecorded,
        "f-1",
        "fact",
        serde_json::json!({ "kind": "loc", "statement": "the repo is 1,342 lines" }),
    );
    let proj = Projection::build(&state.store, &state.project).unwrap();
    assert_eq!(proj.facts.len(), 1);
    assert_eq!(proj.facts[0].id, "f-1");
    assert_eq!(proj.facts[0].kind, "loc");
    assert_eq!(proj.facts[0].recorded_by, "owner");
    assert!(
        !proj.facts[0].recorded_at.is_empty(),
        "point-in-time stamp set"
    );
}

#[test]
fn record_opinion_and_fact_pass_the_gate() {
    // Both pass the gate (note-recording actions have no cross-entity check).
    actions::validate(
        &PmAction::RecordOpinion {
            id: "op-9".into(),
            category: "lesson".into(),
            statement: "Single-owner auth is enough".into(),
            supersedes: None,
        },
        "owner",
        &Projection::default(),
    )
    .unwrap();
    actions::validate(
        &PmAction::RecordFact {
            id: "f-9".into(),
            kind: "events".into(),
            statement: "the log has 164 events".into(),
        },
        "owner",
        &Projection::default(),
    )
    .unwrap();
}

#[test]
fn record_opinion_action_to_events() {
    let state = make_state();
    let cause = cause_for(&state);
    let evs = PmAction::RecordOpinion {
        id: "op-9".into(),
        category: "lesson".into(),
        statement: "Single-owner auth is enough".into(),
        supersedes: None,
    }
    .to_events(&state.project, "owner", &cause, "corr-1");
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].event_type, EventType::OpinionRecorded);
    assert_eq!(evs[0].aggregate.id, "op-9");
}

#[test]
fn pm_can_narrate_opinions_and_facts() {
    // The PM drives actions through to events using its normal loop path —
    // proving RecordOpinion/RecordFact are first-class, not just reducible.
    let state = make_state();
    let cause = cause_for(&state);

    let opinion_ev = PmAction::RecordOpinion {
        id: "op-pm".into(),
        category: "design".into(),
        statement: "The event log is the only authority".into(),
        supersedes: None,
    }
    .to_events(&state.project, "pm", &cause, "corr-1");
    state
        .append(opinion_ev.into_iter().next().unwrap())
        .unwrap();

    let fact_ev = PmAction::RecordFact {
        id: "f-pm".into(),
        kind: "tasks".into(),
        statement: "the board has 3 tasks".into(),
    }
    .to_events(&state.project, "pm", &cause, "corr-1");
    state.append(fact_ev.into_iter().next().unwrap()).unwrap();

    let proj = Projection::build(&state.store, &state.project).unwrap();
    assert_eq!(proj.opinions.len(), 1);
    assert_eq!(proj.opinions[0].recorded_by, "pm");
    assert_eq!(proj.facts.len(), 1);
    assert_eq!(proj.facts[0].recorded_by, "pm");
}
