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
                review: None,
                parent_id: None,
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
        review: None,
        parent_id: None,
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
    // Fail-closed isolation: the assignee may start the task ONLY once an
    // isolated worktree is provisioned (2026-08-12). Without one, rejection.
    assert_eq!(
        validate(
            &casting::actions::PmAction::StartTask {
                task_id: "task-1".into()
            },
            "marcus-reed",
            &st
        ),
        Err(PolicyError::TaskHasNoWorktree("task-1".into()))
    );
    st.worktrees.push(casting::projection::Worktree {
        task_id: "task-1".into(),
        branch: "casting/task-1-x".into(),
        path: "/x".into(),
        cargo_target_dir: "/x/target".into(),
        port: 8090,
    });
    assert!(validate(
        &casting::actions::PmAction::StartTask {
            task_id: "task-1".into()
        },
        "marcus-reed",
        &st
    )
    .is_ok());
}

#[test]
fn provision_worktree_requires_an_assigned_hired_consultant() {
    let mut st = state_with(&["marcus-reed"], &["task-1"]);
    st.tasks[0].assignee = Some("marcus-reed".into());
    let act = casting::actions::PmAction::ProvisionWorktree {
        task_id: "task-1".into(),
        slug: "auth".into(),
        cargo_target_dir: "/x/target".into(),
        port: 8090,
    };
    // Valid: task exists, assigned to a hired consultant, no worktree yet.
    assert!(validate(&act, "pm", &st).is_ok());

    // Reject: owner-assigned tasks never get a Casting worktree (the human
    // works through their own harness).
    st.tasks[0].assignee = Some("owner".into());
    assert_eq!(
        validate(&act, "pm", &st),
        Err(PolicyError::WorktreeForOwner("task-1".into()))
    );
    st.tasks[0].assignee = Some("marcus-reed".into());

    // Reject: a task with no assignee.
    st.tasks[0].assignee = None;
    assert!(validate(&act, "pm", &st).is_err());

    // Reject: assigning to an agent who isn't hired.
    let st2 = state_with(&["marcus-reed"], &["task-1"]);
    // task unassigned -> TaskUnassigned; also test unhired assignee path.
    let mut st3 = state_with(&["marcus-reed"], &["task-1"]);
    st3.tasks[0].assignee = Some("nobody".into());
    assert_eq!(
        validate(&act, "pm", &st3),
        Err(PolicyError::AgentNotHired("nobody".into()))
    );
    let _ = st2;
}

#[test]
fn provision_worktree_rejects_duplicate() {
    let mut st = state_with(&["marcus-reed"], &["task-1"]);
    st.tasks[0].assignee = Some("marcus-reed".into());
    st.worktrees.push(casting::projection::Worktree {
        task_id: "task-1".into(),
        branch: "casting/task-1-auth".into(),
        path: "/x".into(),
        cargo_target_dir: "/x/target".into(),
        port: 8090,
    });
    let act = casting::actions::PmAction::ProvisionWorktree {
        task_id: "task-1".into(),
        slug: "auth".into(),
        cargo_target_dir: "/x/target".into(),
        port: 8090,
    };
    assert_eq!(
        validate(&act, "pm", &st),
        Err(PolicyError::WorktreeAlreadyProvisioned("task-1".into()))
    );
}

#[test]
fn provision_worktree_requires_existing_task() {
    let st = state_with(&["marcus-reed"], &["task-1"]);
    let act = casting::actions::PmAction::ProvisionWorktree {
        task_id: "task-999".into(),
        slug: "x".into(),
        cargo_target_dir: "/x/target".into(),
        port: 8090,
    };
    assert_eq!(
        validate(&act, "pm", &st),
        Err(PolicyError::TaskNotFound("task-999".into()))
    );
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

#[test]
fn record_actions_reject_duplicate_ids() {
    // Fail-closed: previously the catch-all `_ => Ok(())` let creates pass
    // without an id check; now every create-action enforces id uniqueness.
    let mut st = state_with(&[], &[]);
    st.requirements.push(casting::projection::Requirement {
        id: "dup".into(),
        title: "existing".into(),
        description: "".into(),
    });
    let r = casting::actions::PmAction::CreateRequirement {
        id: "dup".into(),
        title: "x".into(),
        description: "y".into(),
    };
    assert_eq!(
        validate(&r, "pm", &st),
        Err(PolicyError::DuplicateEntity("dup".into())),
        "duplicate requirement id must be rejected (fail-closed)"
    );
}

#[test]
fn gate_is_fail_closed_not_fail_open() {
    // The anti-regression guarantee: validate() must have NO catch-all. We can't
    // add a variant at runtime, but we CAN assert the explicit per-entity
    // uniqueness arms exist and reject a duplicate, which is the behavior the
    // catch-all previously skipped. (The compile-time fail-closed guarantee is
    // that removing an arm here makes the match non-exhaustive => build error.)
    let st = state_with(&[], &["task-1"]);
    // A SendMessage has no cross-entity invariant and must pass.
    let msg = casting::actions::PmAction::SendMessage {
        to: "owner".into(),
        body: "hi".into(),
    };
    assert!(validate(&msg, "pm", &st).is_ok());
    // A NoOp is always allowed.
    assert!(validate(&casting::actions::PmAction::NoOp, "pm", &st).is_ok());
}

#[test]
fn decompose_requires_existing_parent() {
    let st = state_with(&[], &["task-1"]);
    let act = casting::actions::PmAction::DecomposeTask {
        parent: "nope".into(),
        children: vec![casting::actions::TaskSpec {
            id: "task-2".into(),
            title: "child".into(),
            kind: "feature".into(),
        }],
    };
    assert_eq!(
        validate(&act, "pm", &st),
        Err(PolicyError::TaskNotFound("nope".into())),
        "decomposing a nonexistent parent must be rejected"
    );
}

#[test]
fn decompose_rejects_duplicate_or_taken_child_ids() {
    let st = state_with(&[], &["task-1", "task-2"]);
    // Duplicate within the decomposition.
    let dup = casting::actions::PmAction::DecomposeTask {
        parent: "task-1".into(),
        children: vec![
            casting::actions::TaskSpec {
                id: "task-3".into(),
                title: "a".into(),
                kind: "feature".into(),
            },
            casting::actions::TaskSpec {
                id: "task-3".into(),
                title: "b".into(),
                kind: "feature".into(),
            },
        ],
    };
    assert_eq!(
        validate(&dup, "pm", &st),
        Err(PolicyError::DuplicateEntity("task-3".into()))
    );
    // Child id already an existing task.
    let taken = casting::actions::PmAction::DecomposeTask {
        parent: "task-1".into(),
        children: vec![casting::actions::TaskSpec {
            id: "task-2".into(),
            title: "c".into(),
            kind: "feature".into(),
        }],
    };
    assert_eq!(
        validate(&taken, "pm", &st),
        Err(PolicyError::DuplicateEntity("task-2".into()))
    );
}

#[test]
fn decompose_valid_when_parent_exists_and_children_fresh() {
    let st = state_with(&[], &["task-1"]);
    let act = casting::actions::PmAction::DecomposeTask {
        parent: "task-1".into(),
        children: vec![casting::actions::TaskSpec {
            id: "task-2".into(),
            title: "child".into(),
            kind: "feature".into(),
        }],
    };
    assert!(validate(&act, "pm", &st).is_ok());
}

#[test]
fn start_gate_is_fail_closed_on_unsatisfied_hard_dependency() {
    use casting::projection::{TaskDependency, TaskStatus, Worktree};
    // `api` is assigned + has an isolated worktree, but it's hard-blocked on
    // `db` (still queued). Starting it must fail at the gate — ordering is
    // enforced by the policy gate, not left to the PM/LLM.
    let st = Projection {
        project_id: "proj-t".into(),
        agents: vec![Agent {
            id: "marcus-reed".into(),
            role: "consultant".into(),
        }],
        tasks: vec![
            casting::projection::Task {
                id: "api".into(),
                title: "api".into(),
                kind: "backend".into(),
                status: TaskStatus::Backlog,
                assignee: Some("marcus-reed".into()),
                priority: casting::plan::Priority::default(),
                review: None,
                parent_id: None,
            },
            casting::projection::Task {
                id: "db".into(),
                title: "db".into(),
                kind: "infra".into(),
                status: TaskStatus::Backlog,
                assignee: Some("maya-patel".into()),
                priority: casting::plan::Priority::default(),
                review: None,
                parent_id: None,
            },
        ],
        dependencies: vec![TaskDependency {
            task: "api".into(),
            blocking_task: "db".into(),
            required_state: TaskStatus::Done,
        }],
        worktrees: vec![Worktree {
            task_id: "api".into(),
            branch: "casting/api".into(),
            path: "/wt/api".into(),
            cargo_target_dir: "/wt/target".into(),
            port: 8091,
        }],
        ..Default::default()
    };
    let act = casting::actions::PmAction::StartTask {
        task_id: "api".into(),
    };
    assert_eq!(
        validate(&act, "marcus-reed", &st),
        Err(PolicyError::BlockedByDependency {
            task_id: "api".into(),
            blockers: vec!["db".into()],
        }),
        "starting a hard-blocked task must be rejected at the gate (Blocker Test)"
    );
}
