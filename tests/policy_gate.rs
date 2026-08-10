//! Unit tests for the PM policy gate (`actions.rs`): the typed action
//! vocabulary and the pure validation layer that sits between any reasoning
//! source (today the scripted PM, tomorrow an LLM) and the event store.
//!
//! These prove BOTH halves of the gate:
//!  1. That valid actions pass (so the scripted PM's plan executes),
//!  2. That invalid actions are REJECTED before they touch any state —
//!     the exact guarantee that protects tokens once a real LLM is wired in.

use casting::actions::{validate, PolicyError};
use casting::projection::{Agent, Projection};

/// A projection with just enough state to exercise cross-entity invariants.
fn state_with(agents: &[&str], tasks: &[&str]) -> Projection {
    Projection {
        project_id: "proj-t".into(),
        agents: agents
            .iter()
            .map(|id| Agent {
                id: id.to_string(),
                role: "consultant".into(),
            })
            .collect(),
        tasks: tasks
            .iter()
            .map(|t| casting::projection::Task {
                id: t.to_string(),
                title: t.to_string(),
                kind: "feature".into(),
                status: casting::projection::TaskStatus::Backlog,
                assignee: None,
                priority: casting::plan::Priority::default(),
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn cannot_hire_an_agent_twice() {
    let st = state_with(&["marcus-reed"], &[]);
    let act = casting::actions::PmAction::HireAgent {
        agent_id: "marcus-reed".into(),
        role: "engineer".into(),
    };
    assert_eq!(
        validate(&act, "system", &st),
        Err(PolicyError::AgentAlreadyHired("marcus-reed".into()))
    );
}

#[test]
fn can_hire_a_new_agent() {
    let st = state_with(&["maya-patel"], &[]);
    let act = casting::actions::PmAction::HireAgent {
        agent_id: "marcus-reed".into(),
        role: "engineer".into(),
    };
    assert!(validate(&act, "system", &st).is_ok());
}

#[test]
fn cannot_assign_a_task_that_does_not_exist() {
    let st = state_with(&["marcus-reed"], &[]);
    let act = casting::actions::PmAction::AssignTask {
        task_id: "task-nope".into(),
        assignee: "marcus-reed".into(),
    };
    assert_eq!(
        validate(&act, "system", &st),
        Err(PolicyError::TaskNotFound("task-nope".into()))
    );
}

#[test]
fn cannot_assign_work_to_an_unhired_agent() {
    // Task exists, but the assignee was never hired.
    let st = state_with(&[], &["task-1"]);
    let act = casting::actions::PmAction::AssignTask {
        task_id: "task-1".into(),
        assignee: "ghost-agent".into(),
    };
    assert_eq!(
        validate(&act, "system", &st),
        Err(PolicyError::AgentNotHired("ghost-agent".into()))
    );
}

#[test]
fn cannot_start_a_missing_task() {
    let st = state_with(&["marcus-reed"], &[]);
    let act = casting::actions::PmAction::StartTask {
        task_id: "nowhere".into(),
    };
    assert_eq!(
        validate(&act, "marcus-reed", &st),
        Err(PolicyError::TaskNotFound("nowhere".into()))
    );
}

#[test]
fn cannot_create_duplicate_task_ids() {
    let st = state_with(&[], &["task-1"]);
    let act = casting::actions::PmAction::CreateTask {
        id: "task-1".into(),
        title: "again".into(),
        kind: "feature".into(),
    };
    assert_eq!(
        validate(&act, "system", &st),
        Err(PolicyError::TaskAlreadyExists("task-1".into()))
    );
}

#[test]
fn a_full_valid_sequence_passes() {
    // Simulate the scripted onboard plan's shape: hire -> create -> assign
    // against a growing projection (what `run_planned` does live).
    let mut st = state_with(&[], &[]);
    assert!(validate(
        &casting::actions::PmAction::HireAgent {
            agent_id: "marcus-reed".into(),
            role: "engineer".into(),
        },
        "system",
        &st
    )
    .is_ok());
    st.agents.push(Agent {
        id: "marcus-reed".into(),
        role: "engineer".into(),
    });
    assert!(validate(
        &casting::actions::PmAction::CreateTask {
            id: "task-1".into(),
            title: "x".into(),
            kind: "feature".into(),
        },
        "system",
        &st
    )
    .is_ok());
    st.tasks.push(casting::projection::Task {
        id: "task-1".into(),
        title: "x".into(),
        kind: "feature".into(),
        status: casting::projection::TaskStatus::Backlog,
        assignee: None,
        priority: casting::plan::Priority::default(),
    });
    assert!(validate(
        &casting::actions::PmAction::AssignTask {
            task_id: "task-1".into(),
            assignee: "marcus-reed".into(),
        },
        "system",
        &st
    )
    .is_ok());
}

#[test]
fn cannot_start_a_task_you_dont_own() {
    // task-1 is assigned to marcus-reed; maya-patel may not start it.
    let mut st = state_with(&["marcus-reed", "maya-patel"], &["task-1"]);
    st.tasks[0].assignee = Some("marcus-reed".into());
    let act = casting::actions::PmAction::StartTask {
        task_id: "task-1".into(),
    };
    assert_eq!(
        validate(&act, "maya-patel", &st),
        Err(PolicyError::NotAssignee {
            task_id: "task-1".into(),
            actor: "maya-patel".into(),
            assignee: "marcus-reed".into(),
        })
    );
}

#[test]
fn assignee_can_start_their_own_task() {
    let mut st = state_with(&["marcus-reed"], &["task-1"]);
    st.tasks[0].assignee = Some("marcus-reed".into());
    let act = casting::actions::PmAction::StartTask {
        task_id: "task-1".into(),
    };
    assert!(validate(&act, "marcus-reed", &st).is_ok());
}

#[test]
fn cannot_complete_an_unassigned_task() {
    // task-1 exists, has no assignee — a non-system actor may not complete it.
    let st = state_with(&["marcus-reed"], &["task-1"]);
    let act = casting::actions::PmAction::CompleteTask {
        task_id: "task-1".into(),
        result: "done".into(),
    };
    assert_eq!(
        validate(&act, "marcus-reed", &st),
        Err(PolicyError::TaskUnassigned("task-1".into()))
    );
}

#[test]
fn system_can_act_on_any_task() {
    // system is trusted and may start/complete/block without being the assignee.
    let mut st = state_with(&["marcus-reed"], &["task-1"]);
    st.tasks[0].assignee = Some("marcus-reed".into());
    assert!(validate(
        &casting::actions::PmAction::StartTask {
            task_id: "task-1".into(),
        },
        "system",
        &st
    )
    .is_ok());
    assert!(validate(
        &casting::actions::PmAction::CompleteTask {
            task_id: "task-1".into(),
            result: "done".into(),
        },
        "system",
        &st
    )
    .is_ok());
}

#[test]
fn actions_round_trip_through_json() {
    // The seam contract: an LLM returns JSON, we parse it to the same typed
    // action. Prove the tag + fields survive both directions.
    let act = casting::actions::PmAction::HireAgent {
        agent_id: "maya-patel".into(),
        role: "QA".into(),
    };
    let json = serde_json::to_value(&act).unwrap();
    let back: casting::actions::PmAction = serde_json::from_value(json).unwrap();
    assert_eq!(act, back);

    let act = casting::actions::PmAction::CreateTask {
        id: "task-7".into(),
        title: "Auth".into(),
        kind: "feature".into(),
    };
    let json = serde_json::to_value(&act).unwrap();
    let back: casting::actions::PmAction = serde_json::from_value(json).unwrap();
    assert_eq!(act, back);
}
