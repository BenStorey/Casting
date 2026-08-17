//! Tests for recording project OPINIONS (subjective knowledge) and FACTS
//! (objective point-in-time measures) as first-class events. Owner concept
//! (2026-08-10): knowledge worth not re-deriving is *opinion*; objective
//! measures are the cases we capture as point-in-time facts.

use casting::actions::{self, PmAction};
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

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
            Actor::Director {
                user_id: "ceo".into(),
            },
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
        Actor::Director {
            user_id: "ceo".into(),
        },
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
fn superseding_flips_status_and_active_view_reports_only_valid() {
    // the director's exact scenario: op-1 and op-2 are about DIFFERENT things;
    // op-2 supersedes op-1; a third op-3 is unrelated. Readers must get the
    // currently-valid set, not everything ever recorded.
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
    append(
        &state,
        EventType::OpinionRecorded,
        "op-2",
        "opinion",
        serde_json::json!({
            "category": "design",
            "statement": "The event log is the only authority",
        }),
    );
    // A third opinion supersedes op-1 (different topic from op-2 entirely).
    append(
        &state,
        EventType::OpinionRecorded,
        "op-3",
        "opinion",
        serde_json::json!({
            "category": "rationale",
            "statement": "FoundationDB fits the ordered-log shape better",
            "supersedes": "op-1",
        }),
    );
    // AND the explicit supersession event flips op-1's status.
    state
        .append(Event::new(
            &state.project,
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::OpinionSuperseded,
            Aggregate {
                kind: "opinion".into(),
                id: "op-1".into(),
            },
            serde_json::json!({ "superseded_by": "op-3" }),
        ))
        .unwrap();

    let proj = Projection::build(&state.store, &state.project).unwrap();
    // All three recorded (audit trail intact).
    assert_eq!(proj.opinions.len(), 3);

    // op-1 is folded to Superseded; op-2 and op-3 stay Active.
    let by_id = |id: &str| proj.opinions.iter().find(|o| o.id == id).unwrap();
    use casting::projection::OpinionStatus;
    assert_eq!(by_id("op-1").status, OpinionStatus::Superseded);
    assert_eq!(by_id("op-2").status, OpinionStatus::Active);
    assert_eq!(by_id("op-3").status, OpinionStatus::Active);

    // Readers asking "what's currently valid" get exactly op-2 and op-3.
    let active: Vec<&str> = proj
        .active_opinions()
        .into_iter()
        .map(|o| o.id.as_str())
        .collect();
    let mut active = active;
    active.sort_unstable();
    assert_eq!(active, vec!["op-2", "op-3"]);

    // Category-scoped: current rationale = op-3 only (op-1 superseded).
    let rationale: Vec<&str> = proj
        .active_opinions_by_category("rationale")
        .into_iter()
        .map(|o| o.id.as_str())
        .collect();
    assert_eq!(rationale, vec!["op-3"]);
}

#[test]
fn supersede_opinion_action_through_gate_and_events() {
    let state = make_state();
    append(
        &state,
        EventType::OpinionRecorded,
        "op-old",
        "opinion",
        serde_json::json!({ "category": "preference", "statement": "old view" }),
    );
    append(
        &state,
        EventType::OpinionRecorded,
        "op-new",
        "opinion",
        serde_json::json!({ "category": "preference", "statement": "new view" }),
    );
    let proj = Projection::build(&state.store, &state.project).unwrap();

    // Supersede passes the gate (both exist + active).
    actions::validate(
        &PmAction::SupersedeOpinion {
            opinion_id: "op-old".into(),
            by_opinion_id: "op-new".into(),
        },
        "director",
        &proj,
        None,
    )
    .unwrap();

    // Guilds against superseding the same opinion or a non-existent one.
    assert!(matches!(
        actions::validate(
            &PmAction::SupersedeOpinion {
                opinion_id: "op-old".into(),
                by_opinion_id: "op-old".into(),
            },
            "owner",
            &proj,
            None,
        ),
        Err(casting::actions::PolicyError::OpinionNotFound(_))
    ));
    assert!(matches!(
        actions::validate(
            &PmAction::SupersedeOpinion {
                opinion_id: "nope".into(),
                by_opinion_id: "op-new".into(),
            },
            "owner",
            &proj,
            None,
        ),
        Err(casting::actions::PolicyError::OpinionNotFound(_))
    ));

    let cause = cause_for(&state);
    let evs = PmAction::SupersedeOpinion {
        opinion_id: "op-old".into(),
        by_opinion_id: "op-new".into(),
    }
    .to_events(&state.project, "owner", &cause, "corr-1");
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].event_type, EventType::OpinionSuperseded);
    assert_eq!(evs[0].aggregate.id, "op-old");

    // Applying it flips op-old to Superseded.
    state.append(evs.into_iter().next().unwrap()).unwrap();
    let after = Projection::build(&state.store, &state.project).unwrap();
    use casting::projection::OpinionStatus;
    let old = after.opinions.iter().find(|o| o.id == "op-old").unwrap();
    assert_eq!(old.status, OpinionStatus::Superseded);
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
            subject: "auth".into(),
            category: "lesson".into(),
            statement: "Single-owner auth is enough".into(),
            supersedes: None,
        },
        "owner",
        &Projection::default(),
        None,
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
        None,
    )
    .unwrap();
}

#[test]
fn record_opinion_action_to_events() {
    let state = make_state();
    let cause = cause_for(&state);
    let evs = PmAction::RecordOpinion {
        id: "op-9".into(),
        subject: "auth".into(),
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
        subject: "architecture".into(),
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
