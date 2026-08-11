//! Tests for the drift reconciler — the cursor-gated "every N events" pass.
//!
//! Owner framing (2026-08-10): knowledge drifts rather than going stale in a
//! burst; keep writes simple and reconcile periodically. The reconciler is its
//! own consumer (a cursor) that wakes every N events, mechanically detects
//! same-subject opinion contradictions, and emits SupersedeOpinion through the
//! gate.

use casting::cursor::{CursorStore, SqliteCursorStore};
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::{OpinionStatus, Projection};
use casting::reconciler;
use casting::sqlite_store::SqliteEventStore;
use casting::store::EventStore;

fn state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-rec").with_reconcile_interval(2)
}

fn append_opinion(state: &AppState, id: &str, subject: &str) {
    state
        .append(Event::new(
            &state.project,
            Actor::Owner,
            EventType::OpinionRecorded,
            Aggregate {
                kind: "opinion".into(),
                id: id.into(),
            },
            serde_json::json!({
                "subject": subject,
                "category": "design",
                "statement": format!("opinion {id} about {subject}"),
            }),
        ))
        .unwrap();
}

#[test]
fn drift_detects_same_subject_duplicates_keeping_latest() {
    let st = state();
    append_opinion(&st, "op-a1", "databases");
    append_opinion(&st, "op-a2", "databases"); // newer, same subject
    append_opinion(&st, "op-b", "auth"); // different subject -> untouched

    let proj = Projection::build(&st.store, &st.project).unwrap();
    let mut drifts = reconciler::drift(&proj);
    drifts.sort_by(|a, b| a.subject.cmp(&b.subject));
    assert_eq!(drifts.len(), 1);
    // The OLDER duplicate is flagged, superseded by the newer one.
    assert_eq!(drifts[0].older_id, "op-a1");
    assert_eq!(drifts[0].by_id, "op-a2");
    assert_eq!(drifts[0].subject, "databases");
}

#[test]
fn drift_ignores_superseded_and_empty_subject() {
    let st = state();
    append_opinion(&st, "op-a1", "databases");
    // op-a1 gets explicitly superseded (no longer Active) -> not re-flagged.
    st.append(Event::new(
        &st.project,
        Actor::Owner,
        EventType::OpinionSuperseded,
        Aggregate {
            kind: "opinion".into(),
            id: "op-a1".into(),
        },
        serde_json::json!({ "superseded_by": "op-a2" }),
    ))
    .unwrap();
    append_opinion(&st, "op-empty", ""); // empty subject -> ungroupable, skipped

    let proj = Projection::build(&st.store, &st.project).unwrap();
    assert!(reconciler::drift(&proj).is_empty());
}

#[test]
fn should_run_is_cursor_gated() {
    let st = state();
    // No events yet -> not due (nothing to reconcile).
    assert!(!reconciler::should_run(&st).unwrap());
    // Append more than interval(2) events -> due.
    append_opinion(&st, "op-1", "db");
    append_opinion(&st, "op-2", "db");
    append_opinion(&st, "op-3", "db");
    assert!(reconciler::should_run(&st).unwrap());
    // After running, the cursor advanced -> not due until the next window.
    // (reconcile below also drives this via run_if_due in the loop test.)
}

#[tokio::test]
async fn reconciler_supersedes_false_duplicates_and_advances_cursor() {
    let st = state();
    append_opinion(&st, "op-a1", "databases");
    append_opinion(&st, "op-a2", "databases"); // contradiction
    append_opinion(&st, "op-b", "auth"); // unrelated

    let edited = reconciler::reconcile(&st).await.unwrap();
    // Op-a2 (same subject) supersedes op-a1; op-b untouched.
    assert_eq!(edited, 1);

    let proj = Projection::build(&st.store, &st.project).unwrap();
    let by_id = |id: &str| proj.opinions.iter().find(|o| o.id == id).unwrap();
    assert_eq!(by_id("op-a1").status, OpinionStatus::Superseded);
    assert_eq!(by_id("op-a2").status, OpinionStatus::Active);
    assert_eq!(by_id("op-b").status, OpinionStatus::Active);

    // The reconciler cursor advanced to the head, so it's no longer immediately
    // due until the next window elapses.
    let cursor = st
        .cursors
        .get(&st.project, reconciler::RECONCILER_CONSUMER)
        .unwrap();
    assert_eq!(
        cursor.last_seen,
        st.store.latest_sequence(&st.project).unwrap()
    );
    assert!(!reconciler::should_run(&st).unwrap());
}

#[tokio::test]
async fn reconcile_is_idempotent() {
    let st = state();
    append_opinion(&st, "op-a1", "databases");
    append_opinion(&st, "op-a2", "databases");

    assert_eq!(reconciler::reconcile(&st).await.unwrap(), 1);
    // Second pass: no new drift (op-a1 already superseded) -> no events.
    assert_eq!(reconciler::reconcile(&st).await.unwrap(), 0);
    let proj = Projection::build(&st.store, &st.project).unwrap();
    let by_id = |id: &str| proj.opinions.iter().find(|o| o.id == id).unwrap();
    assert_eq!(by_id("op-a2").status, OpinionStatus::Active);
}

#[tokio::test]
async fn run_if_due_only_fires_after_interval() {
    let st = state().with_reconcile_interval(3);
    // Below the interval -> not due, no-op.
    append_opinion(&st, "op-1", "db");
    append_opinion(&st, "op-2", "db");
    assert_eq!(reconciler::run_if_due(&st).await.unwrap(), 0);

    // Third event crosses the threshold -> due, runs. Three same-subject
    // opinions collapse to one: op-1 superseded by op-2, then op-2 by op-3
    // (2 supersedions), leaving only op-3 Active.
    append_opinion(&st, "op-3", "db");
    let edited = reconciler::run_if_due(&st).await.unwrap();
    assert_eq!(edited, 2);
    let proj = Projection::build(&st.store, &st.project).unwrap();
    let by_id = |id: &str| proj.opinions.iter().find(|o| o.id == id).unwrap();
    assert_eq!(by_id("op-1").status, OpinionStatus::Superseded);
    assert_eq!(by_id("op-2").status, OpinionStatus::Superseded);
    assert_eq!(by_id("op-3").status, OpinionStatus::Active);
}
