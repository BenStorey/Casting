//! Tests for Task review status (roadmap item 5): work doesn't count as Done
//! until someone reviews it. ReadyForReview -> InReview; TaskReviewed approved
//! -> Done (review recorded) or rejected -> Working (rework).

use casting::actions::{validate, PmAction, PolicyError};
use casting::cursor::SqliteCursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::{Projection, TaskStatus};
use casting::sqlite_store::SqliteEventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-review")
}

fn hire(state: &AppState, id: &str, role: &str) {
    state
        .append(Event::new(
            "proj-review",
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: id.into(),
            },
            serde_json::json!({ "role": role }),
        ))
        .unwrap();
}

fn task_in_review(state: &AppState, id: &str) {
    // Create -> assign -> start -> complete -> ready-for-review.
    for (et, data) in [
        (
            EventType::TaskCreated,
            serde_json::json!({ "title": id, "kind": "feature" }),
        ),
        (
            EventType::TaskAssigned,
            serde_json::json!({ "assignee": "marcus-reed" }),
        ),
        (EventType::TaskStarted, serde_json::json!({})),
        (
            EventType::TaskCompleted,
            serde_json::json!({ "result": "done" }),
        ),
        (
            EventType::TaskReadyForReview,
            serde_json::json!({ "reviewer": "maya-patel" }),
        ),
    ] {
        state
            .append(Event::new(
                "proj-review",
                Actor::Agent {
                    id: "marcus-reed".into(),
                },
                et,
                Aggregate {
                    kind: "task".into(),
                    id: id.into(),
                },
                data,
            ))
            .unwrap();
    }
}

#[test]
fn ready_for_review_moves_task_to_in_review() {
    let state = make_state();
    task_in_review(&state, "task-a");
    let proj = Projection::build(&state.store, "proj-review").unwrap();
    let t = proj.tasks.iter().find(|t| t.id == "task-a").unwrap();
    assert_eq!(t.status, TaskStatus::InReview);
    assert!(t.review.is_none(), "not yet reviewed");
}

#[test]
fn approved_review_records_verdict_and_marks_done() {
    let state = make_state();
    hire(&state, "maya-patel", "QA");
    task_in_review(&state, "task-a");
    // QA approves.
    state
        .append(Event::new(
            "proj-review",
            Actor::Agent {
                id: "maya-patel".into(),
            },
            EventType::TaskReviewed,
            Aggregate {
                kind: "task".into(),
                id: "task-a".into(),
            },
            serde_json::json!({ "approved": true, "note": "looks good" }),
        ))
        .unwrap();

    let proj = Projection::build(&state.store, "proj-review").unwrap();
    let t = proj.tasks.iter().find(|t| t.id == "task-a").unwrap();
    assert_eq!(t.status, TaskStatus::Done);
    let r = t.review.as_ref().expect("review recorded");
    assert_eq!(r.reviewer, "maya-patel");
    assert!(r.approved);
    assert_eq!(r.note, "looks good");
}

#[test]
fn rejected_review_sends_back_to_working_for_rework() {
    let state = make_state();
    hire(&state, "maya-patel", "QA");
    task_in_review(&state, "task-a");
    // QA rejects; the task goes back to Working (rework).
    state
        .append(Event::new(
            "proj-review",
            Actor::Agent {
                id: "maya-patel".into(),
            },
            EventType::TaskReviewed,
            Aggregate {
                kind: "task".into(),
                id: "task-a".into(),
            },
            serde_json::json!({ "approved": false, "note": "redo it" }),
        ))
        .unwrap();

    let proj = Projection::build(&state.store, "proj-review").unwrap();
    let t = proj.tasks.iter().find(|t| t.id == "task-a").unwrap();
    assert_eq!(t.status, TaskStatus::Working, "rejected -> rework");
    let r = t.review.as_ref().expect("rejection recorded");
    assert!(!r.approved);
    assert_eq!(r.note, "redo it");
}

// --- Gate ---

/// Build a projection with a single created task for gate checks.
fn projection_with_task() -> (AppState, Projection, String) {
    let state = make_state();
    hire(&state, "marcus-reed", "Engineer");
    hire(&state, "maya-patel", "QA");
    state
        .append(Event::new(
            "proj-review",
            Actor::System,
            EventType::TaskCreated,
            Aggregate {
                kind: "task".into(),
                id: "task-1".into(),
            },
            serde_json::json!({ "title": "T", "kind": "feature" }),
        ))
        .unwrap();
    let proj = Projection::build(&state.store, "proj-review").unwrap();
    (state, proj, "task-1".into())
}

#[test]
fn request_review_requires_working_task_and_hired_reviewer() {
    let (state, mut proj, task_id) = projection_with_task();
    // Not-working task: no reviewer yet -> reject.
    let act = PmAction::RequestReview {
        task_id: task_id.clone(),
        reviewer: "maya-patel".into(),
    };
    let err = validate(&act, "marcus-reed", &proj).expect_err("must be working to review");
    assert!(matches!(
        err,
        PolicyError::TaskNotFound(_) | PolicyError::TaskUnassigned(_)
    ));

    // Give it an assignee so the actor is the assignee; still not in review.
    state
        .append(Event::new(
            "proj-review",
            Actor::System,
            EventType::TaskAssigned,
            Aggregate {
                kind: "task".into(),
                id: "task-1".into(),
            },
            serde_json::json!({ "assignee": "marcus-reed" }),
        ))
        .unwrap();
    proj = Projection::build(&state.store, "proj-review").unwrap();
    let act = PmAction::RequestReview {
        task_id: "task-1".into(),
        reviewer: "maya-patel".into(),
    };
    // Assignee (marcus) submitting his own (assigned but not completed) work is
    // allowed; the gate check is assignee==who && reviewer hired.
    assert!(validate(&act, "marcus-reed", &proj).is_ok());
    // Unhired reviewer is rejected.
    let act = PmAction::RequestReview {
        task_id: "task-1".into(),
        reviewer: "ghost".into(),
    };
    let err = validate(&act, "marcus-reed", &proj).expect_err("unhired reviewer");
    assert!(matches!(err, PolicyError::AgentNotHired(_)));
}

#[test]
fn review_task_requires_in_review_status() {
    let (_state, proj, task_id) = projection_with_task();
    let act = PmAction::ReviewTask {
        task_id,
        approved: true,
        note: Some("ok".into()),
    };
    let err = validate(&act, "maya-patel", &proj).expect_err("cannot review a non-in-review task");
    assert!(matches!(err, PolicyError::TaskNotInReview(_)));
}
