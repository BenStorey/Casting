//! Tests for the tiered merge policy (2026-08-14): a task's merge_authority
//! (self vs pm) decides whether it can complete directly to Done (self) or must
//! pass through the PM's review (pm), plus the SetMergeAuthority escape hatch.

use casting::actions::{validate, PmAction, PolicyError};
use casting::types::{MergeAuthority, Task, TaskStatus};
use casting::projection::{Agent, Projection};
use casting::cursor::SqliteCursorStore;
use casting::pm::AppState;
use casting::sqlite_store::SqliteEventStore;

/// A project with one task in Working, assigned to `agent`, with the given
/// merge authority. `agent` is the only hired agent (so it's a valid assignee
/// and the only one who may complete the task).
fn project(agent_id: &str, authority: MergeAuthority) -> Projection {
    Projection {
        project_id: "proj-ma".into(),
        agents: vec![Agent {
            id: agent_id.into(),
            role: "consultant".into(),
        }],
        tasks: vec![Task {
            id: "task-1".into(),
            title: "t".into(),
            kind: "feature".into(),
            status: TaskStatus::Working,
            assignee: Some(agent_id.into()),
            merge_authority: authority,
            priority: casting::plan::Priority::Medium,
            review: None,
            parent_id: None,
        }],
        ..Default::default()
    }
}

#[test]
fn self_merge_task_can_complete_directly_to_done() {
    let st = project("lead-programmer", MergeAuthority::SelfMerge);
    // The assignee's CompleteTask is the fast path (trivial → CI-gated direct done).
    assert!(
        validate(
            &PmAction::CompleteTask {
                task_id: "task-1".into(),
                result: "done".into(),
            },
            "lead-programmer",
            &st
        )
        .is_ok(),
        "self-merge task completes directly"
    );
}

#[test]
fn pm_merge_task_cannot_complete_directly_it_must_be_reviewed() {
    let st = project("lead-programmer", MergeAuthority::PmMerge);
    let err = validate(
        &PmAction::CompleteTask {
            task_id: "task-1".into(),
            result: "done".into(),
        },
        "lead-programmer",
        &st,
    )
    .expect_err("pm-merge task must not skip the PM's review");
    assert!(matches!(err, PolicyError::PmMergeRequiresReview(_)));
}

#[test]
fn pm_merge_task_still_can_submit_to_a_real_reviewer() {
    let mut st = project("lead-programmer", MergeAuthority::PmMerge);
    // Re-route to a second hired reviewer (test-engineer) for the submit step.
    st.agents.push(Agent {
        id: "test-engineer".into(),
        role: "consultant".into(),
    });
    assert!(
        validate(
            &PmAction::RequestReview {
                task_id: "task-1".into(),
                reviewer: "test-engineer".into(),
            },
            "lead-programmer",
            &st
        )
        .is_ok(),
        "pm-merge work submits through RequestReview"
    );
}

#[test]
fn escape_hatch_reclassifies_self_to_pm_by_the_pm() {
    // A self-merge task that grew in scope gets escalated by the PM.
    let st = project("lead-programmer", MergeAuthority::SelfMerge);
    assert!(
        validate(
            &PmAction::SetMergeAuthority {
                task_id: "task-1".into(),
                merge_authority: MergeAuthority::PmMerge,
            },
            "pm",
            &st
        )
        .is_ok(),
        "PM may reclassify merge authority"
    );
}

#[test]
fn only_pm_owner_may_reclassify_merge_authority() {
    let st = project("lead-programmer", MergeAuthority::SelfMerge);
    // A plain consultant cannot change the merge gate.
    let err = validate(
        &PmAction::SetMergeAuthority {
            task_id: "task-1".into(),
            merge_authority: MergeAuthority::PmMerge,
        },
        "lead-programmer",
        &st,
    )
    .expect_err("a consultant must not set merge authority");
    assert!(matches!(err, PolicyError::ActionNotAuthorized(_)));
}

#[tokio::test]
async fn reclassification_is_event_sourced_and_lasts() {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj-ma");

    // Create + assign (SelfMerge) a task.
    let created = PmAction::CreateTask {
        id: "task-e".into(),
        title: "t".into(),
        kind: "feature".into(),
    }
    .to_events("proj-ma", "pm", &cause(&state, "c1"), "c1");
    for e in created {
        state.append(e).unwrap();
    }
    let assigned = PmAction::AssignTask {
        task_id: "task-e".into(),
        assignee: "lead-programmer".into(),
        merge_authority: MergeAuthority::SelfMerge,
    }
    .to_events("proj-ma", "pm", &cause(&state, "c2"), "c2");
    for e in assigned {
        state.append(e).unwrap();
    }

    // Reclassify self -> pm via the escape hatch.
    let reclass = PmAction::SetMergeAuthority {
        task_id: "task-e".into(),
        merge_authority: MergeAuthority::PmMerge,
    }
    .to_events("proj-ma", "pm", &cause(&state, "c3"), "c3");
    assert_eq!(
        reclass[0].event_type,
        casting::event::EventType::MergeAuthorityChanged
    );
    for e in reclass {
        state.append(e).unwrap();
    }

    // The projection reflects the pm-merge decision.
    let proj = Projection::build(&state.store, "proj-ma").unwrap();
    let task = proj.tasks.iter().find(|t| t.id == "task-e").unwrap();
    assert_eq!(task.merge_authority, MergeAuthority::PmMerge);
}

fn cause(state: &AppState, id: &str) -> casting::event::Event {
    casting::event::Event::new(
        &state.project,
        casting::event::Actor::Agent { id: "pm".into() },
        casting::event::EventType::MessageSent,
        casting::event::Aggregate {
            kind: "message".into(),
            id: id.into(),
        },
        serde_json::json!({}),
    )
}