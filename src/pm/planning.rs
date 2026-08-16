//! Deterministic scripted PM planning — the plan *builders* only.
//!
//! Extracted out of `pm.rs` (2026-08-14, de-monolith pass) to shrink the PM
//! control loop's coordination surface. This module owns the pure/static
//! plan-construction logic: given plain inputs (an owner message, a decided
//! cause, a decision policy, a `&AppState` for projection/workspace reads) it
//! returns the SAME typed `Vec<PlannedAction>` a provider would otherwise
//! emit (docs/ADDENDUM.md §16). The control loop in `pm.rs` feeds these
//! through the policy gate unchanged — moving house doesn't change the plans it
//! ships.
//!
//! Everything here is behavior-identical to its former home; it only needs
//! `AppState` for snapshot-aware projection reads (`AppState::projection`) and
//! the optional workspace, never for field layout. It also hosts the tiny
//! orchestrator-audit event builders (`OrchestrationRun` / `PlanActionRejected`)
//! so the loop sites in `pm.rs` don't re-liter the plan-aggregate boilerplate.
//!
//! The old scripted planning functions (`plan_onboard`, `plan_acknowledge`,
//! `plan_owner_decision`) have been removed — they were the demo tape.
//! All planning now goes through the `Orchestrator` trait (real LLM or
//! `MockOrchestrator` in tests).

use crate::actions::PmAction;
use crate::event::{Actor, Event};
use crate::pm::AppState;

/// Building an `OrchestrationRun` audit event (aggregate kind `"plan"`, shared
/// `run-{seq}` correlation). Deduped plan-aggregate telemetry: kept as a tiny
/// helper so `pm.rs` doesn't repeat the plan-aggregate boilerplate.
pub(crate) fn orchestration_run_event(
    project: &str,
    correlation: &str,
    body: serde_json::Value,
) -> Event {
    Event::new(
        project,
        Actor::System,
        crate::event::EventType::OrchestrationRun,
        crate::event::Aggregate {
            kind: "plan".into(),
            id: correlation.into(),
        },
        body,
    )
}

/// Building a `PlanActionRejected` audit event (the policy gate refused an
/// action during `run_planned`). Same plan-aggregate shape as the
/// orchestration audit; factored out with it.
pub(crate) fn plan_rejected_event(
    project: &str,
    correlation: &str,
    body: serde_json::Value,
) -> Event {
    Event::new(
        project,
        Actor::System,
        crate::event::EventType::PlanActionRejected,
        crate::event::Aggregate {
            kind: "plan".into(),
            id: correlation.into(),
        },
        body,
    )
}

/// Insert `ProvisionWorktree` actions before each `StartTask` in a plan,
/// unless the task is assigned to the owner or system. This is the
/// deterministic worktree elaborator — the platform's structural isolation
/// guarantee, kept as a deterministic rewriter so both scripted and LLM
/// plans automatically get worktree provisioning without each producer
/// having to reason about ports.
pub(crate) fn insert_worktree_provisions(
    state: &AppState,
    plan: &mut Vec<crate::pm::PlannedAction>,
    claimed_ports: &mut std::collections::HashSet<u16>,
) {
    let projection = state
        .projection()
        .unwrap_or_else(|_| crate::projection::Projection::default());
    let mut i = 0;
    while i < plan.len() {
        if let (_, PmAction::StartTask { task_id }) = &plan[i] {
            let assignee = plan.iter().find_map(|(_, a)| {
                if let PmAction::AssignTask {
                    task_id: tid,
                    assignee,
                    ..
                } = a
                {
                    if tid == task_id && assignee != "owner" {
                        Some(assignee.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            });
            // Also check projection for already-assigned tasks.
            let assignee = assignee.or_else(|| {
                projection
                    .tasks
                    .iter()
                    .find(|t| t.id == *task_id)
                    .and_then(|t| t.assignee.clone())
                    .filter(|a| a != "owner")
            });
            if let Some(ref who) = assignee {
                let port = find_free_port(&projection, claimed_ports);
                claimed_ports.insert(port);
                let cargo_target_dir = format!(".casting/worktrees/{who}-0/target");
                let prov = (
                    "pm".into(),
                    PmAction::ProvisionWorktree {
                        task_id: task_id.clone(),
                        assignee: who.clone(),
                        slug: String::new(),
                        cargo_target_dir,
                        slot: 0,
                        port,
                    },
                );
                plan.insert(i, prov);
                i += 1; // skip the just-inserted provision
            }
        }
        i += 1;
    }
}

/// Find a free port from the worktree port pool, not already claimed in
/// `claimed_in_plan` or used by an existing provisioned worktree.
fn find_free_port(
    projection: &crate::projection::Projection,
    claimed_in_plan: &std::collections::HashSet<u16>,
) -> u16 {
    let used_in_projection: std::collections::HashSet<u16> =
        projection.worktrees.iter().map(|w| w.port).collect();
    let base = crate::projection::port::worktree_base_port();
    let span = crate::projection::port::WORKTREE_PORT_POOL;
    (base..base.saturating_add(span))
        .find(|p| !used_in_projection.contains(p) && !claimed_in_plan.contains(p))
        .unwrap_or(crate::projection::port::DEFAULT_WORKTREE_BASE_PORT)
}

/// Actors who have actionable work in the current projection: assignee
/// consultants with non-done tasks, plus the PM (who reviews and manages).
/// Returns actor ids (strings) in a deterministic order.
pub(crate) fn actors_with_work(projection: &crate::projection::Projection) -> Vec<String> {
    use crate::projection::TaskStatus;
    let mut actors: Vec<String> = Vec::new();

    for task in &projection.tasks {
        if task.status == TaskStatus::Done {
            continue;
        }
        if let Some(ref assignee) = task.assignee {
            if assignee == "owner" {
                continue;
            }
            // Only include actors who are actually hired.
            if !projection.agents.iter().any(|a| a.id == *assignee) {
                continue;
            }
            if !actors.contains(assignee) {
                actors.push(assignee.clone());
            }
        }
    }

    // Also include the PM if any task is InReview (so the PM/reviewer acts).
    if projection
        .tasks
        .iter()
        .any(|t| t.status == TaskStatus::InReview)
        && !actors.contains(&"pm".to_string())
    {
        actors.push("pm".into());
    }

    actors
}
