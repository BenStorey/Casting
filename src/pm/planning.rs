//! Deterministic scripted PM planning — the plan *builders* only.
//!
//! Extracted out of `pm.rs` (2026-08-14, de-monolith pass) to shrink the PM
//! control loop's coordination surface. This module owns the pure/static
//! plan-construction logic: given plain inputs (a director message, a decided
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
/// unless the task is assigned to the director or system. This is the
/// deterministic worktree elaborator — the platform's structural isolation
/// guarantee, kept as a deterministic rewriter so both scripted and LLM
/// plans automatically get worktree provisioning without each producer
/// having to reason about ports.
///
/// Parent worktree reuse (2026-08-16): if a task already has a parent whose
/// worktree is (or will be) provisioned, we skip provisioning a *new* one —
/// playbook children share the parent's worktree.
pub(crate) fn insert_worktree_provisions(
    state: &AppState,
    plan: &mut Vec<crate::pm::PlannedAction>,
    claimed_ports: &mut std::collections::HashSet<u16>,
) {
    let projection = state
        .projection()
        .unwrap_or_else(|_| crate::projection::Projection::default());
    let mut claimed_slots: std::collections::HashSet<usize> =
        projection.worktrees.iter().map(|w| w.slot).collect();
    let mut i = 0;
    while i < plan.len() {
        if let (_, PmAction::StartTask { task_id }) = &plan[i] {
            // --- Parent worktree reuse check ---
            // If this task has a parent that already has (or will have) a
            // worktree, skip provisioning — playbook children share it.
            let has_parent_worktree = projection
                .tasks
                .iter()
                .find(|t| t.id == *task_id)
                .and_then(|t| t.parent_id.as_ref())
                .and_then(|pid| {
                    // Check existing worktrees
                    if projection
                        .worktrees
                        .iter()
                        .any(|w| w.task_id.as_deref() == Some(pid.as_str()))
                    {
                        return Some(true);
                    }
                    // Check the plan — is there a ProvisionWorktree for the parent?
                    if plan.iter().any(|(_, a)| {
                        matches!(a, PmAction::ProvisionWorktree { task_id: tid, .. } if tid == pid)
                    }) {
                        return Some(true);
                    }
                    None
                })
                .unwrap_or(false);

            if has_parent_worktree {
                i += 1;
                continue;
            }

            let assignee = plan.iter().find_map(|(_, a)| {
                if let PmAction::AssignTask {
                    task_id: tid,
                    assignee,
                    ..
                } = a
                {
                    if tid == task_id && assignee != "director" {
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
                    .filter(|a| a != "director")
            });
            if let Some(ref who) = assignee {
                let port = find_free_port(&projection, claimed_ports);
                claimed_ports.insert(port);
                let slot = (0..).find(|s| !claimed_slots.contains(s)).unwrap_or(0);
                claimed_slots.insert(slot);
                let cargo_target_dir = format!(".casting/worktrees/{who}-{slot}/target");
                let prov = (
                    projection.pm_id().to_string(),
                    PmAction::ProvisionWorktree {
                        task_id: task_id.clone(),
                        assignee: who.clone(),
                        slug: String::new(),
                        cargo_target_dir,
                        slot,
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

/// Expand `ApplyPlaybook` actions onto the deterministic task-graph
/// primitives: `DecomposeTask` + `BlockTaskOn` chain + `AssignTask` per step
/// + `ProvisionWorktree` for the parent.
///
/// This runs AFTER `insert_worktree_provisions` so worktree provisioning for
/// the parent task is already present. The playbook elaborator emits child
/// tasks that share the parent's worktree (checked via `has_parent_worktree`).
pub(crate) fn expand_playbooks(
    state: &AppState,
    plan: &mut Vec<crate::pm::PlannedAction>,
    claimed_ports: &mut std::collections::HashSet<u16>,
) {
    let projection = state
        .projection()
        .unwrap_or_else(|_| crate::projection::Projection::default());
    let mut i = 0;
    while i < plan.len() {
        let (_who, action) = &plan[i];
        let (playbook_id, parent_task_id, recipe) = match action {
            PmAction::ApplyPlaybook {
                playbook_id,
                parent_task_id,
                recipe,
                ..
            } => (playbook_id.clone(), parent_task_id.clone(), recipe),
            _ => {
                i += 1;
                continue;
            }
        };

        // Derive the assignee from the plan: find an AssignTask for the
        // parent task, or look it up in the projection.
        let assignee = plan.iter().find_map(|(_, a)| {
            if let PmAction::AssignTask {
                task_id: tid,
                assignee,
                ..
            } = a
            {
                if tid == &parent_task_id && assignee != "director" {
                    Some(assignee.clone())
                } else {
                    None
                }
            } else {
                None
            }
        });
        let assignee = assignee.or_else(|| {
            projection
                .tasks
                .iter()
                .find(|t| t.id == parent_task_id)
                .and_then(|t| t.assignee.clone())
                .filter(|a| a != "director")
        });
        let Some(assignee) = assignee else {
            log::warn!(
                "[planning] expand_playbooks: cannot determine assignee for \
                 parent '{parent_task_id}' — replacing ApplyPlaybook with NoOp"
            );
            plan[i] = (projection.pm_id().to_string(), PmAction::NoOp);
            i += 1;
            continue;
        };

        // Determine the steps to use.
        let steps: Vec<crate::consultants::playbook::PlaybookStep> = if let Some(ref adhoc) = recipe
        {
            // Ad-hoc recipe: steps are directly on the AdHocRecipe.
            // Validate that steps are non-empty (quick validation).
            if adhoc.steps.is_empty() {
                log::warn!(
                    "[planning] expand_playbooks: ad-hoc recipe for \
                         '{parent_task_id}' has no steps — replacing with NoOp"
                );
                plan[i] = (projection.pm_id().to_string(), PmAction::NoOp);
                i += 1;
                continue;
            }
            adhoc.steps.clone()
        } else {
            // Load from the consultant registry via state.
            let found = state
                .consultants
                .playbook(&playbook_id)
                .map(|(_consultant, pb)| pb.steps.clone());

            match found {
                Some(steps) => steps,
                None => {
                    log::warn!(
                        "[planning] expand_playbooks: playbook '{playbook_id}' \
                             not found in consultant registry — replacing with NoOp"
                    );
                    plan[i] = (projection.pm_id().to_string(), PmAction::NoOp);
                    i += 1;
                    continue;
                }
            }
        };

        // Build child tasks from the playbook steps.
        let children: Vec<crate::actions::TaskSpec> = steps
            .iter()
            .map(|s| crate::actions::TaskSpec {
                id: format!("{parent_task_id}/{}", s.id),
                title: s.title.clone(),
                kind: "playbook-step".into(),
            })
            .collect();

        // Build the expanded actions to replace ApplyPlaybook.
        let who_pm = projection.pm_id().to_string();
        let mut expanded: Vec<crate::pm::PlannedAction> = Vec::new();

        // 1. DecomposeTask: register all children under the parent.
        expanded.push((
            who_pm.clone(),
            PmAction::DecomposeTask {
                parent: parent_task_id.clone(),
                children: children.clone(),
            },
        ));

        // 2. BlockTaskOn chain: step N waits on step N-1 Done.
        for pair in steps.windows(2) {
            let first = &pair[0];
            let second = &pair[1];
            expanded.push((
                who_pm.clone(),
                PmAction::BlockTaskOn {
                    task_id: format!("{parent_task_id}/{}", second.id),
                    blocking_task_id: format!("{parent_task_id}/{}", first.id),
                    required_state: crate::types::TaskStatus::Done,
                },
            ));
        }

        // 3. AssignTask for each child to the same assignee.
        for child in &children {
            expanded.push((
                who_pm.clone(),
                PmAction::AssignTask {
                    task_id: child.id.clone(),
                    assignee: assignee.clone(),
                    merge_authority: crate::types::MergeAuthority::default(),
                },
            ));
        }

        // 4. ProvisionWorktree for the parent if none exists yet.
        //    Check if there's already a worktree for this task (from the
        //    worktree elaborator) and if not, add one.
        let has_worktree = projection
            .worktrees
            .iter()
            .any(|w| w.task_id.as_deref() == Some(&parent_task_id))
            || plan.iter().any(|(_, a)| {
                matches!(a, PmAction::ProvisionWorktree { task_id: tid, .. } if *tid == parent_task_id)
            });

        if !has_worktree && assignee != "director" {
            let port = find_free_port(&projection, claimed_ports);
            claimed_ports.insert(port);
            let claimed_slots: std::collections::HashSet<usize> =
                projection.worktrees.iter().map(|w| w.slot).collect();
            let slot = (0..).find(|s| !claimed_slots.contains(s)).unwrap_or(0);
            let cargo_target_dir = format!(".casting/worktrees/{assignee}-{slot}/target");
            expanded.push((
                who_pm.clone(),
                PmAction::ProvisionWorktree {
                    task_id: parent_task_id.clone(),
                    assignee: assignee.clone(),
                    slug: String::new(),
                    cargo_target_dir,
                    slot,
                    port,
                },
            ));
        }

        // Replace the ApplyPlaybook with the expanded sequence.
        plan.splice(i..=i, expanded);
        i += children.len() + 1; // skip past everything we inserted
    }
}

/// Actors who have actionable work in the current projection: assignee
/// consultants with non-done tasks, plus the PM (who reviews and manages).
/// Returns actor ids (strings) in a deterministic order.
///
/// Skips Backlog tasks that are blocked by hard dependencies: if a task is
/// still Backlog and `projection.blocked_by(&task.id)` returns non-empty,
/// the actor should not be woken — attempting StartTask would be rejected.
pub(crate) fn actors_with_work(projection: &crate::projection::Projection) -> Vec<String> {
    use crate::projection::TaskStatus;
    let mut actors: Vec<String> = Vec::new();

    for task in &projection.tasks {
        if task.status == TaskStatus::Done {
            continue;
        }
        if let Some(ref assignee) = task.assignee {
            if assignee == "director" {
                continue;
            }
            // Only include actors who are actually hired. The PM is a special
            // case — they can self-assign tasks via the chat-interface playbook
            // and are never in the agents list (not hirable).
            if !projection.is_pm(assignee) && !projection.agents.iter().any(|a| a.id == *assignee) {
                continue;
            }
            // Skip Backlog tasks that are blocked by unresolved dependencies.
            // The actor has no actionable work because StartTask would be
            // rejected at the policy gate.
            if task.status == TaskStatus::Backlog && !projection.blocked_by(&task.id).is_empty() {
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
        && !actors.contains(&projection.pm_id().to_string())
    {
        actors.push(projection.pm_id().to_string());
    }

    actors
}
