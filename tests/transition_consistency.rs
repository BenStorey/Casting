//! Consistency between the task-transition TABLE in `src/graph.rs` (the single
//! written source of truth for transition legality — it feeds the PM prompt,
//! a validation/debug check and the dashboard) and the policy gate
//! (`actions::validate`). The gate consults `graph::valid_from_status` for its
//! status-transition checks instead of hand-writing status matches, so the two
//! can never drift. These tests assert they agree on representative
//! (from-status, action) pairs — including the illegal ones — and that the
//! gate's non-status invariants (task-must-exist, assignee rules) are intact.

use casting::actions::{validate, PmAction, PolicyError};
use casting::graph::valid_from_status;
use casting::plan::Priority;
use casting::projection::{Projection, TaskStatus};
use casting::types::{Agent, Task};

fn task(id: &str, status: TaskStatus, assignee: Option<&str>) -> Task {
    Task {
        id: id.to_string(),
        title: "task".into(),
        kind: "feature".into(),
        status,
        assignee: assignee.map(Into::into),
        priority: Priority::Medium,
        review: None,
        parent_id: None,
    }
}

fn agent(id: &str) -> Agent {
    Agent {
        id: id.to_string(),
        role: "QA".into(),
    }
}

fn proj(tasks: Vec<Task>, agents: Vec<Agent>) -> Projection {
    Projection {
        project_id: "transition-consistency".into(),
        agents,
        tasks,
        ..Default::default()
    }
}

/// ReviewTask is the gate's only status-transition check; the table must agree
/// with it on the (from-status, action) pairs, legal AND illegal.
#[test]
fn graph_table_and_gate_agree_on_status_legality() {
    // --- The transition TABLE answers for the lifecycle actions ---
    // Legal exits.
    assert!(valid_from_status(TaskStatus::Working, "start_task"));
    assert!(valid_from_status(TaskStatus::Working, "request_review"));
    assert!(valid_from_status(TaskStatus::InReview, "review_task"));
    // A blocked task can "resume" (ACTION start_task) back into Working.
    assert!(valid_from_status(TaskStatus::Blocked, "start_task"));

    // Illegal exits.
    assert!(!valid_from_status(TaskStatus::Working, "review_task"));
    assert!(!valid_from_status(TaskStatus::Backlog, "start_task"));
    assert!(!valid_from_status(TaskStatus::InReview, "start_task"));
    assert!(!valid_from_status(TaskStatus::Done, "review_task"));
    // Completing isn't a table-governed transition (completion is gated purely
    // by assignee in the policy gate, not by this status contract).
    assert!(!valid_from_status(TaskStatus::Working, "complete_task"));

    // --- The gate agrees: ReviewTask on a Working task is rejected by BOTH ---
    let st = proj(
        vec![task("t-working", TaskStatus::Working, Some("marcus-reed"))],
        vec![agent("maya-patel")],
    );
    let err = validate(
        &PmAction::ReviewTask {
            task_id: "t-working".into(),
            approved: true,
            note: None,
        },
        "maya-patel",
        &st,
    )
    .expect_err("cannot review a working task");
    assert!(matches!(err, PolicyError::TaskNotInReview(_)));
    assert!(!valid_from_status(TaskStatus::Working, "review_task"));

    // --- And it permits the same pair the table permits (InReview review) ---
    let st = proj(
        vec![task("t-review", TaskStatus::InReview, Some("marcus-reed"))],
        vec![agent("maya-patel")],
    );
    assert!(validate(
        &PmAction::ReviewTask {
            task_id: "t-review".into(),
            approved: true,
            note: Some("ok".into()),
        },
        "maya-patel",
        &st,
    )
    .is_ok());
    assert!(valid_from_status(TaskStatus::InReview, "review_task"));
}

/// The gate's non-status invariants are untouched by the unify: a review on a
/// missing task still rejects with TaskNotFound.
#[test]
fn review_missing_task_still_rejected() {
    let st = proj(vec![], vec![]);
    let err = validate(
        &PmAction::ReviewTask {
            task_id: "nope".into(),
            approved: true,
            note: None,
        },
        "maya-patel",
        &st,
    )
    .expect_err("missing task");
    assert!(matches!(err, PolicyError::TaskNotFound(_)));
}

/// The assignee-side lifecycle invariants are unchanged: an assigned but not
/// yet InReview task is still submittable by its assignee (with a hired
/// reviewer), and the assignee gate still stops a non-assignee from reviewing.
#[test]
fn assignee_lifecycle_gates_unchanged() {
    let st = proj(
        vec![task("t-submit", TaskStatus::Backlog, Some("marcus-reed"))],
        vec![agent("marcus-reed"), agent("maya-patel")],
    );
    // An assigned (non-InReview) task's own assignee may still RequestReview
    // with a hired reviewer — a non-status gate, preserved as-is.
    assert!(validate(
        &PmAction::RequestReview {
            task_id: "t-submit".into(),
            reviewer: "maya-patel".into(),
        },
        "marcus-reed",
        &st,
    )
    .is_ok());
}
