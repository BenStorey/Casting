//! Tests for the human-as-consultant delivery model (owner 2026-08-10):
//! a task can be assigned to the OWNER (the human), who executes and delivers
//! it (possibly working through their own harness). Distinct from hired agents.

use casting::cursor::SqliteCursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::{Projection, TaskStatus};
use casting::sqlite_store::SqliteEventStore;

fn state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-human")
}

fn hire_engineer(st: &AppState) {
    st.append(Event::new(
        &st.project,
        Actor::System,
        EventType::AgentHired,
        Aggregate {
            kind: "agent".into(),
            id: "marcus-reed".into(),
        },
        serde_json::json!({ "role": "engineer", "agent_id": "marcus-reed" }),
    ))
    .unwrap();
}

fn create_task(st: &AppState, id: &str, title: &str) {
    st.append(Event::new(
        &st.project,
        Actor::System,
        EventType::TaskCreated,
        Aggregate {
            kind: "task".into(),
            id: id.into(),
        },
        serde_json::json!({ "title": title, "kind": "implement", "requirement_id": "req-1" }),
    ))
    .unwrap();
}

#[test]
fn assign_task_to_owner_is_valid() {
    let st = state();
    hire_engineer(&st);
    create_task(&st, "task-1", "Build the API");

    let proj = Projection::build(&st.store, &st.project).unwrap();
    // Assigning to the owner is allowed (human-as-consultant).
    let ok = casting::actions::validate(
        &casting::actions::PmAction::AssignTask {
            task_id: "task-1".into(),
            assignee: casting::actions::OWNER.into(),
            merge_authority: casting::types::MergeAuthority::PmMerge,
        },
        "pm",
        &proj,
    );
    assert!(ok.is_ok(), "owner should be a valid assignee: {ok:?}");

    // Assigning to an unhired agent is still rejected.
    let bad = casting::actions::validate(
        &casting::actions::PmAction::AssignTask {
            task_id: "task-1".into(),
            assignee: "ghost-agent".into(),
            merge_authority: casting::types::MergeAuthority::PmMerge,
        },
        "pm",
        &proj,
    );
    assert!(bad.is_err(), "unhired agent must still be rejected");
}

#[test]
fn owner_can_start_and_complete_their_own_task() {
    let st = state();
    create_task(&st, "task-1", "Build the API");
    // Assign to the owner directly via the event.
    st.append(Event::new(
        &st.project,
        Actor::System,
        EventType::TaskAssigned,
        Aggregate {
            kind: "task".into(),
            id: "task-1".into(),
        },
        serde_json::json!({ "assignee": casting::actions::OWNER }),
    ))
    .unwrap();

    // Owner (who == "owner") can start and complete it.
    let proj = Projection::build(&st.store, &st.project).unwrap();
    assert!(casting::actions::validate(
        &casting::actions::PmAction::StartTask {
            task_id: "task-1".into()
        },
        casting::actions::OWNER,
        &proj
    )
    .is_ok());
    assert!(casting::actions::validate(
        &casting::actions::PmAction::CompleteTask {
            task_id: "task-1".into(),
            result: "done".into()
        },
        casting::actions::OWNER,
        &proj
    )
    .is_ok());

    // An agent (marcus-reed) is NOT the assignee — must be rejected.
    hire_engineer(&st);
    let proj2 = Projection::build(&st.store, &st.project).unwrap();
    assert!(
        casting::actions::validate(
            &casting::actions::PmAction::StartTask {
                task_id: "task-1".into()
            },
            "marcus-reed",
            &proj2
        )
        .is_err(),
        "an agent must not act on the owner's task"
    );
}

#[test]
fn owner_delivery_shows_in_projection() {
    let st = state();
    create_task(&st, "task-1", "Build the API");
    st.append(Event::new(
        &st.project,
        Actor::System,
        EventType::TaskAssigned,
        Aggregate {
            kind: "task".into(),
            id: "task-1".into(),
        },
        serde_json::json!({ "assignee": casting::actions::OWNER }),
    ))
    .unwrap();
    st.append(Event::new(
        &st.project,
        Actor::Owner,
        EventType::TaskCompleted,
        Aggregate {
            kind: "task".into(),
            id: "task-1".into(),
        },
        serde_json::json!({}),
    ))
    .unwrap();

    let proj = Projection::build(&st.store, &st.project).unwrap();
    let t = &proj.tasks[0];
    assert_eq!(t.assignee.as_deref(), Some("owner"));
    assert_eq!(t.status, TaskStatus::Done);
}
