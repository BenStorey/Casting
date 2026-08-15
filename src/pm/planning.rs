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

use crate::actions::{PmAction, OWNER};
use crate::event::{Actor, Event};
use crate::pm::AppState;
use crate::pm::DecisionClass;

/// Stable agent roster the simulated company uses (moved here with the plan
/// builders; `PM_CONSUMER` remains the loop's cursor consumer in `pm.rs`).
const AGENT_ENG: &str = "marcus-reed";
const AGENT_QA: &str = "maya-patel";

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

/// Build a `ProvisionWorktree` action for a task, allocating a distinct port
/// from the pool (the lowest free one not already used by a provisioned
/// worktree OR allocated earlier in the same plan — plans can provision several
/// worktrees before any event executes, so we must exclude already-claimed
/// ports of this plan too). Isolated workspaces are the platform's structural
/// guarantee (2026-08-12) — the action the PM plans so a consultant is handed
/// a ready desk, never asked to "remember" to isolate.
fn plan_worktree_provision(
    state: &AppState,
    task_id: &str,
    assignee: &str,
    slug: &str,
    claimed_in_plan: &mut std::collections::HashSet<u16>,
) -> PmAction {
    let projection = state
        .projection()
        .unwrap_or_else(|_| crate::projection::Projection::default());
    let used_in_projection: std::collections::HashSet<u16> =
        projection.worktrees.iter().map(|w| w.port).collect();
    let base = crate::projection::port::worktree_base_port();
    let span = crate::projection::port::WORKTREE_PORT_POOL;
    let port = (base..base.saturating_add(span))
        .find(|p| !used_in_projection.contains(p) && !claimed_in_plan.contains(p))
        .unwrap_or(crate::projection::port::DEFAULT_WORKTREE_BASE_PORT);
    claimed_in_plan.insert(port);

    // Select the first free slot for this assignee (a slot is free when its
    // worktree is not bound to an active task). Defaults to 0 if the assignee
    // has no provisioned worktrees yet (the first provision creates slot 0).
    let assignee_slots: Vec<usize> = projection
        .worktrees
        .iter()
        .filter(|w| w.consultant == assignee)
        .map(|w| w.slot)
        .collect();
    let max_concurrent = state
        .consultants
        .by_id(assignee)
        .map(|c| c.max_concurrent)
        .unwrap_or(1);
    let slot = (0..max_concurrent)
        .find(|s| !assignee_slots.contains(s))
        .unwrap_or(0);

    let cargo_target_dir = match &state.workspace {
        Some(ws) => ws
            .consultant_worktree_path(assignee, slot)
            .join("target")
            .to_string_lossy()
            .into_owned(),
        None => format!(".casting/worktrees/{assignee}-{slot}/target"),
    };
    PmAction::ProvisionWorktree {
        task_id: task_id.to_string(),
        assignee: assignee.to_string(),
        slug: slug.to_string(),
        cargo_target_dir,
        slot,
        port,
    }
}

/// First owner message: onboard the company and kick off a build. Plans the
/// whole sequence as actions; the gate lets each through as the projection
/// grows.
pub(crate) fn plan_onboard(
    state: &AppState,
    cause: &Event,
    body: &str,
    policy: &crate::pm::DecisionPolicy,
) -> Vec<crate::pm::PlannedAction> {
    let title = if body.trim().is_empty() {
        "the product".to_string()
    } else {
        body.trim().to_string()
    };

    // The testing-library decision is auto-decided by the PM ONLY when the
    // (event-sourced) policy routes it to the agent. If the owner has
    // escalated it to Ask, the PM proposes it and leaves it in the owner's
    // inbox — no auto-decision, no follow-up task.
    let testing_lib_decider = policy.resolve(DecisionClass::TestingLibrary).decider();

    // Onboard the working team: hire the default cast members by role, but SKIP
    // anyone the setup engine already hired. If setup explicitly chose a cast
    // (any non-PM agent exists), we DON'T top-up — the owner's chosen team
    // stands. The PM is hired separately at seed, so the cast here is the
    // working team.
    let already_hired: Vec<String> = state
        .projection()
        .ok()
        .map(|p| p.agents.iter().map(|a| a.id.clone()).collect())
        .unwrap_or_default();
    let has_existing_cast = already_hired.iter().any(|id| id != "pm"); // any non-PM agent = setup chose the cast
    let cast_hires: Vec<crate::pm::PlannedAction> = if has_existing_cast {
        Vec::new()
    } else {
        crate::workspace::DEFAULT_CAST
            .iter()
            .filter(|m| !already_hired.iter().any(|id| id == m.agent_id))
            .map(|m| {
                let role = crate::workspace::role_by_id(m.role_id).unwrap_or_else(|| {
                    panic!("default cast role {} missing from catalog", m.role_id)
                });
                (
                    "system".into(),
                    PmAction::HireAgent {
                        agent_id: m.agent_id.into(),
                        role: role.title.into(),
                    },
                )
            })
            .collect()
    };

    let mut plan: Vec<crate::pm::PlannedAction> = vec![
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::CreateRequirement {
                id: format!("req-{}", cause.event_id),
                title: title.clone(),
                description: body.to_string(),
            },
        ),
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::SendMessage {
                to: "owner".into(),
                body: format!("Understood — \u{201c}{title}\u{201d}. I've broken this into tasks and brought in Marcus (engineering) and Maya (QA). Stand by."),
            },
        ),
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::CreateTask {
                id: "task-design".into(),
                title: format!("Design {title}"),
                kind: "feature".into(),
            },
        ),
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::AssignTask { task_id: "task-design".into(), assignee: AGENT_ENG.into(), merge_authority: crate::types::MergeAuthority::SelfMerge },
        ),
        (
            AGENT_ENG.into(),
            PmAction::StartTask { task_id: "task-design".into() },
        ),
        (
            AGENT_ENG.into(),
            PmAction::CompleteTask { task_id: "task-design".into(), result: format!("Designed {title}") },
        ),
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::CreateTask {
                id: "task-core".into(),
                title: format!("Implement {title} core"),
                kind: "feature".into(),
            },
        ),
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::AssignTask { task_id: "task-core".into(), assignee: AGENT_ENG.into(), merge_authority: crate::types::MergeAuthority::PmMerge },
        ),
        (AGENT_ENG.into(), PmAction::StartTask { task_id: "task-core".into() }),
        (
            AGENT_QA.into(),
            PmAction::CreateObservation {
                id: "obs-1".into(),
                severity: "info".into(),
                subject: "HTTPS not enabled in the scaffold".into(),
                body: "Noted during review. Won't fix now, but worth a task later.".into(),
                pm_action_required: false,
            },
        ),
        // Marcus submits the core work for review; the PM routes it to QA.
        (
            AGENT_ENG.into(),
            PmAction::RequestReview {
                task_id: "task-core".into(),
                reviewer: AGENT_QA.into(),
            },
        ),
        (
            AGENT_QA.into(),
            PmAction::ReviewTask {
                task_id: "task-core".into(),
                approved: true,
                note: Some("Core looks solid — marcus integrates and ships".into()),
            },
        ),
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::CreateTask {
                id: "task-qa".into(),
                title: "Set up automated tests".into(),
                kind: "feature".into(),
            },
        ),
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::AssignTask { task_id: "task-qa".into(), assignee: AGENT_QA.into(), merge_authority: crate::types::MergeAuthority::SelfMerge },
        ),
        (AGENT_QA.into(), PmAction::StartTask { task_id: "task-qa".into() }),
        (
            AGENT_QA.into(),
            PmAction::CompleteTask { task_id: "task-qa".into(), result: "Test suite passing".into() },
        ),
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::ProposeDecision {
                id: "decision-db".into(),
                subject: "Database choice".into(),
                options: serde_json::json!({
                    "A": "PostgreSQL — robust, more infra, approx $18",
                    "B": "SQLite — dead simple, zero infra, approx $9"
                }),
                recommendation: "A".into(),
                // Resolve the involvement from the configured (event-sourced)
                // policy; Database defaults to Ask -> routes to the OWNER.
                class: DecisionClass::Database,
                involvement: policy.resolve(DecisionClass::Database),
            },
        ),
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::SendMessage {
                to: "owner".into(),
                body: "We need one call from you: which database for this build? I recommend A (PostgreSQL) for headroom, but B (SQLite) is zero-infra and cheaper.".into(),
            },
        ),
        // Delegated authority demo: choosing the testing library is a Pm-class
        // decision, so the PM decides it itself — DecisionProposed then
        // DecisionMade (actor = PM), no owner question, but fully recorded.
        (
            crate::pm::PM_CONSUMER.into(),
            PmAction::ProposeDecision {
                id: "decision-testing-lib".into(),
                subject: "Automated-testing library".into(),
                options: serde_json::json!({
                    "A": "pytest — batteries included",
                    "B": "cargo test — keep it in Rust"
                }),
                recommendation: "B".into(),
                class: DecisionClass::TestingLibrary,
                involvement: policy.resolve(DecisionClass::TestingLibrary),
            },
        ),
    ];
    // Hire the default cast first, before any work is planned.
    plan.splice(0..0, cast_hires);

    // Auto-decide the testing-library decision ONLY when the policy routes it
    // to the agent. If the owner escalated it to Ask, leave it open in their
    // inbox (Proposed) with no follow-up until they rule.
    if testing_lib_decider == crate::pm::Decider::Agent {
        plan.push((
            crate::pm::PM_CONSUMER.into(),
            PmAction::MakeDecision {
                decision_id: "decision-testing-lib".into(),
                approved: true,
                note: Some("PM: choosing cargo test, keep the toolchain single-language".into()),
            },
        ));
        plan.push((
            crate::pm::PM_CONSUMER.into(),
            PmAction::CreateTask {
                id: "task-testing-lib".into(),
                title: "Set up testing library (cargo test)".into(),
                kind: "feature".into(),
            },
        ));
    }

    let mut claimed_ports: std::collections::HashSet<u16> = std::collections::HashSet::new();

    // Feature-Mode promotion (opt-in): if the requirement is cross-cutting, the
    // PM decomposes it into parallel children and adds Blocker-Test hard edges
    // so ordering is enforced by the gate, not left to chance. The created
    // parent is the join point; children are kicked in parallel. Default off to
    // keep the canonical demo flat + existing tests green (flip once proven).
    if state.decompose {
        if let Some(dec) =
            crate::projection::graph::should_decompose("task-feature", "feature", &title)
        {
            plan.push((
                crate::pm::PM_CONSUMER.into(),
                PmAction::CreateTask {
                    id: dec.feature_id.clone(),
                    title: format!("Feature: {title}"),
                    kind: "feature".into(),
                },
            ));
            plan.push((
                crate::pm::PM_CONSUMER.into(),
                PmAction::DecomposeTask {
                    parent: dec.feature_id.clone(),
                    children: dec.children.clone(),
                },
            ));
            for (dependent, blocker, required) in &dec.hard_edges {
                plan.push((
                    crate::pm::PM_CONSUMER.into(),
                    PmAction::BlockTaskOn {
                        task_id: dependent.clone(),
                        blocking_task_id: blocker.clone(),
                        required_state: *required,
                    },
                ));
            }
            // Drive every child through the SAME lifecycle as any other task
            // (assign -> start -> complete -> submit -> review -> done), so
            // subtasks are first-class tasks, not a second-class entity. The
            // join resolves when all children reach Done. Children are
            // sequenced topologically (blockers before dependents) so a
            // hard-blocked child's StartTask only runs after its blocker
            // completes — the gate enforces it, so this is the only way the
            // blocked child can ever reach Done.
            let mut remaining: Vec<&crate::actions::TaskSpec> = dec.children.iter().collect();
            while !remaining.is_empty() {
                let idx = remaining
                    .iter()
                    .position(|c| {
                        !dec.hard_edges.iter().any(|(dep, blk, _)| {
                            dep == &c.id && remaining.iter().any(|r| r.id == *blk)
                        })
                    })
                    .expect("decomposition must be acyclic");
                let child = remaining.remove(idx);
                let assignee = AGENT_ENG; // all children to the engineer; QA reviews
                let reviewer = AGENT_QA;
                plan.push((
                    crate::pm::PM_CONSUMER.into(),
                    PmAction::AssignTask {
                        task_id: child.id.clone(),
                        assignee: assignee.into(),
                        merge_authority: crate::types::MergeAuthority::SelfMerge,
                    },
                ));
                plan.push((
                    assignee.into(),
                    PmAction::StartTask {
                        task_id: child.id.clone(),
                    },
                ));
                plan.push((
                    assignee.into(),
                    PmAction::CompleteTask {
                        task_id: child.id.clone(),
                        result: format!("{} done", child.title),
                    },
                ));
                plan.push((
                    assignee.into(),
                    PmAction::RequestReview {
                        task_id: child.id.clone(),
                        reviewer: reviewer.into(),
                    },
                ));
                plan.push((
                    reviewer.into(),
                    PmAction::ReviewTask {
                        task_id: child.id.clone(),
                        approved: true,
                        note: Some("approved".into()),
                    },
                ));
            }
        }
    }

    // Structural isolation (2026-08-12): before any consultant STARTS a task,
    // the platform provisions its isolated worktree (own branch/build-target/
    // port). Walk the plan and insert ProvisionWorktree ahead of each StartTask
    // for a task assigned to a hired agent (not the owner). The gate already
    // rejects StartTask without a worktree, so this is what makes onboarding
    // actually work.
    let mut i = 0;
    while i < plan.len() {
        if let (_, PmAction::StartTask { task_id }) = &plan[i] {
            // Find the assignee from the matching AssignTask action.
            let assignee = plan.iter().find_map(|(_, a)| {
                if let PmAction::AssignTask {
                    task_id: tid,
                    assignee,
                    ..
                } = a
                {
                    if tid == task_id && assignee != OWNER {
                        Some(assignee.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            });
            let assigned_to_consultant = assignee.is_some();
            if assigned_to_consultant {
                let prov = (
                    crate::pm::PM_CONSUMER.into(),
                    plan_worktree_provision(
                        state,
                        task_id,
                        assignee.as_deref().unwrap_or("unknown"),
                        "",
                        &mut claimed_ports,
                    ),
                );
                plan.insert(i, prov);
                i += 1; // skip the just-inserted provision
            }
        }
        i += 1;
    }

    plan
}

/// Owner just messaged but we already have requirements — acknowledge politely.
pub(crate) fn plan_acknowledge(cause: &Event) -> Vec<crate::pm::PlannedAction> {
    let body = cause
        .data
        .get("body")
        .and_then(|b| b.as_str())
        .unwrap_or("");
    vec![(
        crate::pm::PM_CONSUMER.into(),
        PmAction::SendMessage {
            to: "owner".into(),
            body: format!("Noted: \u{201c}{body}\u{201d}. It's on the backlog — I'll fold it into the next build pass."),
        },
    )]
}

/// The owner ruled on a proposed decision — plan the verdict's consequences.
pub(crate) fn plan_owner_decision(
    state: &AppState,
    cause: &Event,
) -> Vec<crate::pm::PlannedAction> {
    let decision_id = cause.aggregate.id.clone();
    let approved = cause
        .data
        .get("approved")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let note = cause
        .data
        .get("note")
        .and_then(|b| b.as_str())
        .unwrap_or("");
    let subject = cause
        .data
        .get("subject")
        .and_then(|b| b.as_str())
        .unwrap_or("your decision");

    let mut out = vec![(
        crate::pm::PM_CONSUMER.into(),
        PmAction::SendMessage {
            to: "owner".into(),
            body: if approved {
                format!("Great — \u{201c}{subject}\u{201d} approved{}. I'll drive the implementation now.", fmt_note(note))
            } else {
                format!("Understood — \u{201c}{subject}\u{201d} was declined{}. Discarding that option.", fmt_note(note))
            },
        },
    )];

    if approved {
        // If this decision was a GovernanceChange (PM proposed a directive
        // change), applying it is the OWNER's prerogative: only the owner may
        // author directives. The owner just approved it via DecisionMade, so we
        // author the directive change AS the owner — the approval is authority.
        let governance = ApprovedGovernanceChange::from_decision(state, &decision_id);
        if let Some(gov) = governance {
            out.push((
                "owner".into(),
                PmAction::CreateDirective {
                    id: gov.directive_id.clone(),
                    kind: gov.kind,
                    statement: gov.statement,
                    scope: gov.scope,
                    strength: gov.strength,
                    supersedes: gov.supersedes.clone(),
                },
            ));
            if let Some(superseded) = gov.supersedes {
                out.push((
                    "owner".into(),
                    PmAction::SupersedeDirective {
                        directive_id: superseded,
                        by_directive_id: gov.directive_id.clone(),
                    },
                ));
            }
        }

        // A PM-proposed consultant hire (AddConsultant class) is applied on
        // owner approval: the owner said yes, so the hire proceeds.
        let consultant = approved_consultant_role(state, &decision_id);
        if let Some(role_id) = consultant {
            out.push((
                "system".into(),
                PmAction::HireAgent {
                    agent_id: format!("{role_id}-1"),
                    role: crate::workspace::role_by_id(&role_id)
                        .map(|r| r.title.to_string())
                        .unwrap_or_else(|| role_id.clone()),
                },
            ));
        }

        out.push((
            crate::pm::PM_CONSUMER.into(),
            PmAction::CreateTask {
                id: format!("task-adopt-{decision_id}"),
                title: format!("Adopt {subject} (owner-approved)"),
                kind: "feature".into(),
            },
        ));
        out.push((
            crate::pm::PM_CONSUMER.into(),
            PmAction::AssignTask {
                task_id: format!("task-adopt-{decision_id}"),
                assignee: AGENT_ENG.into(),
                merge_authority: crate::types::MergeAuthority::SelfMerge,
            },
        ));
        out.push((
            AGENT_ENG.into(),
            PmAction::StartTask {
                task_id: format!("task-adopt-{decision_id}"),
            },
        ));
        out.push((
            AGENT_ENG.into(),
            PmAction::CompleteTask {
                task_id: format!("task-adopt-{decision_id}"),
                result: format!("Adopted {subject}"),
            },
        ));
    }

    out
}

/// A GovernanceChange decision that the owner approved: the directive change to
/// apply, authored as the owner. Parsed from the DecisionProposed's `options`.
struct ApprovedGovernanceChange {
    directive_id: String,
    kind: crate::runtime::directive::DirectiveKind,
    statement: String,
    scope: Vec<String>,
    strength: crate::runtime::directive::DirectiveStrength,
    supersedes: Option<String>,
}

impl ApprovedGovernanceChange {
    /// Rebuild the projection, find the decision, and if it's an approved
    /// GovernanceChange, extract the proposed directive change.
    fn from_decision(state: &AppState, decision_id: &str) -> Option<Self> {
        // Use AppState::projection() (snapshot-aware) — the single projection
        // entry point. Never rebuild directly from the store.
        let proj = state.projection().ok()?;
        let dec = proj.decisions.iter().find(|d| d.id == decision_id)?;
        if dec.class != crate::pm::DecisionClass::GovernanceChange {
            return None;
        }
        let change = dec.options.get("governance_change")?;
        let kind: crate::runtime::directive::DirectiveKind =
            serde_json::from_value(change.get("kind")?.clone()).ok()?;
        let strength: crate::runtime::directive::DirectiveStrength =
            serde_json::from_value(change.get("strength")?.clone()).ok()?;
        let scope: Vec<String> = serde_json::from_value(change.get("scope")?.clone()).ok()?;
        Some(ApprovedGovernanceChange {
            directive_id: format!("directive-{decision_id}"),
            kind,
            statement: change.get("statement")?.as_str()?.to_string(),
            scope,
            strength,
            supersedes: change
                .get("supersedes")
                .and_then(|v| v.as_str())
                .map(String::from),
        })
    }
}

fn fmt_note(note: &str) -> String {
    if note.is_empty() {
        String::new()
    } else {
        format!(" (\u{201c}{note}\u{201d})")
    }
}

/// An AddConsultant decision that the owner approved: the role to hire.
/// Parsed from the DecisionProposed's `options`.
fn approved_consultant_role(state: &AppState, decision_id: &str) -> Option<String> {
    // Single projection entry point (snapshot-aware).
    let proj = state.projection().ok()?;
    let dec = proj.decisions.iter().find(|d| d.id == decision_id)?;
    if dec.class != crate::pm::DecisionClass::AddConsultant {
        return None;
    }
    dec.options
        .get("consultant")?
        .get("role_id")?
        .as_str()
        .map(str::to_string)
}
