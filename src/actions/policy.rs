//! The policy gate: validation of a proposed action against the projection, and
//! the `PolicyError` rejection vocabulary.
use super::action::{
    advisor_actor_id, is_advisor_actor, is_pm_actor, is_valid_assignee, pm_actor_id, PmAction,
    DIRECTOR,
};
use crate::consultants::ConsultantRegistry;
use crate::pm::policy;
use crate::projection::Projection;

/// Reason a proposed action was rejected by the policy gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    /// Attempting to hire someone already in the company.
    AgentAlreadyHired(String),
    /// Creating a task whose id already exists.
    TaskAlreadyExists(String),
    /// Acting on a task id that does not exist.
    TaskNotFound(String),
    /// Assigning work to an agent who has not been hired.
    AgentNotHired(String),
    /// Assigning work to a reserved SPECIAL role (Advisor) — they
    /// advise, never take implementation tasks.
    SpecialRoleNotAssignable(String),
    /// Reviewing a task that isn't currently in review.
    TaskNotInReview(String),
    /// A task is not in the correct status for the action (replaces the old
    /// misattributed TaskAlreadyExists for status-mismatch errors).
    TaskStatusError(String),
    /// Starting/completing/blocking a task that has not been assigned yet.
    TaskUnassigned(String),
    /// Starting/completing/blocking a task by someone other than its assignee.
    NotAssignee {
        task_id: String,
        actor: String,
        assignee: String,
    },
    /// A `pm`-merge task cannot be completed straight to Done by a consultant —
    /// it must pass through the PM's review first (tiered merge policy).
    PmMergeRequiresReview(String),
    /// The actor lacks authority for a PM/director-only action.
    ActionNotAuthorized(String),
    /// The authority-downgrade guard fired: a decision was proposed with less
    /// director involvement than its class's policy requires (from `policy.rs`).
    /// A producer may never under-claim director involvement — it would silently
    /// bypass the human.
    AuthorityDowngrade {
        class: crate::pm::DecisionClass,
        required: crate::pm::OwnerInvolvement,
        claimed: crate::pm::OwnerInvolvement,
    },
    /// Making a decision (resolving it) on one that does not exist.
    DecisionNotFound(String),
    /// Making a decision on one not yet proposed / already decided.
    DecisionNotOpen(String),
    /// Re-proposing a decision whose subject already has an OPEN decision
    /// (reactive anti-thrash: the PM must not accumulate duplicate open
    /// decisions on the same subject).
    DecisionAlreadyOpen(String),
    /// Resolving/updating a risk that does not exist.
    RiskNotFound(String),
    /// Acting on a directive that does not exist.
    DirectiveNotFound(String),
    /// A referenced opinion doesn't exist (or isn't supersede-able).
    OpinionNotFound(String),
    /// A plain agent (not director/PM/system) trying to change governance.
    DirectiveAuthority(String),
    /// Hiring/proposing a role that isn't in the catalog.
    UnknownRole(String),
    /// Creating an entity whose id already exists (fail-closed id uniqueness for
    /// all create actions, not just tasks/agents).
    DuplicateEntity(String),
    /// A non-authoritative actor (not director, or not system for a watchdog
    /// pause) trying to change the harness guards (budget / pause / resume).
    /// Budget + resume are director-only; pause also permits the system watchdog.
    GuardAuthority(String),
    /// Starting a task that has no provisioned worktree (fail-closed isolation:
    /// a consultant cannot work un-isolated — the platform provisions the
    /// workspace at summon). "Task X has no isolated worktree".
    TaskHasNoWorktree(String),
    /// Provisioning a worktree for a task that already has one.
    WorktreeAlreadyProvisioned(String),
    /// Provisioning a worktree for a task assigned to the director (the human
    /// works through their own harness, not a Casting worktree).
    WorktreeForOwner(String),
    /// Starting a task whose hard dependencies aren't satisfied yet (the
    /// Blocker Test: a task can't begin until its blockers reach their
    /// required state). \"Task X is blocked by [Y, Z]\".
    BlockedByDependency {
        task_id: String,
        blockers: Vec<String>,
    },
    /// Referencing a playbook that doesn't exist in any consultant's catalog.
    PlaybookNotFound(String),
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyError::AgentAlreadyHired(id) => {
                write!(f, "cannot hire {id}: agent already in the company")
            }
            PolicyError::TaskAlreadyExists(id) => {
                write!(f, "cannot create task {id}: already exists")
            }
            PolicyError::TaskNotFound(id) => write!(f, "cannot act on task {id}: no such task"),
            PolicyError::AgentNotHired(id) => {
                write!(f, "cannot assign task to {id}: not hired")
            }
            PolicyError::SpecialRoleNotAssignable(id) => {
                write!(f, "cannot assign task to {id}: special role, not assignable")
            }
            PolicyError::TaskNotInReview(id) => {
                write!(f, "cannot review task {id}: it is not in review")
            }
            PolicyError::TaskStatusError(msg) => write!(f, "{msg}"),
            PolicyError::TaskUnassigned(id) => {
                write!(f, "cannot act on task {id}: no assignee yet")
            }
            PolicyError::NotAssignee {
                task_id,
                actor,
                assignee,
            } => {
                write!(
                    f,
                    "cannot act on task {task_id}: {actor} is not the assignee ({assignee})"
                )
            }
            PolicyError::PmMergeRequiresReview(id) => write!(
                f,
                "cannot complete task {id} directly: it is a pm-merge task and must pass the PM's review first"
            ),
            PolicyError::ActionNotAuthorized(who) => {
                write!(f, "{who} lacks authority for this action")
            }
            PolicyError::AuthorityDowngrade {
                class,
                required,
                claimed,
            } => write!(
                f,
                "authority downgrade: decision class {class:?} requires {required:?} \
                 owner involvement, but the producer claimed {claimed:?}"
            ),
            PolicyError::DecisionNotFound(id) => {
                write!(f, "cannot resolve decision {id}: no such decision")
            }
            PolicyError::DecisionNotOpen(id) => {
                write!(
                    f,
                    "cannot resolve decision {id}: not open (proposed, unresolved)"
                )
            }
            PolicyError::DecisionAlreadyOpen(subject) => write!(
                f,
                "cannot re-propose decision on '{subject}': an open decision on this subject already exists"
            ),
            PolicyError::RiskNotFound(id) => write!(f, "cannot resolve risk {id}: no such risk"),
            PolicyError::DirectiveNotFound(id) => {
                write!(f, "cannot act on directive {id}: no such directive")
            }
            PolicyError::OpinionNotFound(id) => {
                write!(f, "cannot supersede opinion {id}: no such active opinion")
            }
            PolicyError::DirectiveAuthority(who) => {
                write!(
                    f,
                    "{who} lacks authority to change project governance (directives)"
                )
            }
            PolicyError::UnknownRole(role) => write!(f, "unknown role in the cast catalog: {role}"),
            PolicyError::DuplicateEntity(id) => write!(f, "cannot create {id}: id already exists"),
            PolicyError::GuardAuthority(who) => write!(
                f,
                "{who} lacks authority to change the harness guards (budget/pause/resume)"
            ),
            PolicyError::TaskHasNoWorktree(id) => write!(
                f,
                "cannot start task {id}: no isolated worktree provisioned (the platform provisions it at summon)"
            ),
            PolicyError::WorktreeAlreadyProvisioned(id) => {
                write!(f, "cannot provision worktree for task {id}: one already exists")
            }
            PolicyError::WorktreeForOwner(id) => write!(
                f,
                "cannot provision worktree for task {id}: assigned to the director (the human works through their own harness)"
            ),
            PolicyError::BlockedByDependency { task_id, blockers } => write!(
                f,
                "cannot start task {task_id}: waiting on unsatisfied dependency/dependencies [{}]",
                blockers.join(", ")
            ),
            PolicyError::PlaybookNotFound(id) => {
                write!(f, "playbook '{id}' not found in any consultant's catalog")
            }
        }
    }
}

impl std::error::Error for PolicyError {}

/// Validate one action, performed by `who`, against the current projection.
/// Pure and infallible on the store — returns `Ok(())` when the action may
/// proceed.
///
/// `who` is the label from a `PlannedAction` ("system", "director", or an agent
/// id). StartTask/CompleteTask/BlockTask additionally require that `who` IS
/// the task's assignee — the gate stops the wrong agent (or an LLM mistake)
/// from mutating someone else's task.
pub fn validate(
    action: &PmAction,
    who: &str,
    state: &Projection,
    registry: Option<&ConsultantRegistry>,
) -> Result<(), PolicyError> {
    match action {
        PmAction::HireAgent { agent_id, .. } => {
            // The PM/Advisor special roles can never be hired as task-doers;
            // they are fixed co-ordinator / adviser actors. The PM is excluded
            // from the blocked-assignee set to allow self-assignment via the
            // chat-interface playbook, but it is still not hirable. Both are
            // identified by ROLE (not by id), so the id is never a folder name.
            if pm_actor_id(registry) == agent_id || advisor_actor_id(registry) == agent_id {
                return Err(PolicyError::SpecialRoleNotAssignable(agent_id.clone()));
            }
            if state.agents.iter().any(|a| a.id == *agent_id) {
                Err(PolicyError::AgentAlreadyHired(agent_id.clone()))
            } else {
                check_pm_authority(who, registry)
            }
        }
        // CreateTask and DecomposeTask both guard id freshness.
        PmAction::CreateTask { id, .. } => {
            check_pm_authority(who, registry)?;
            if state.tasks.iter().any(|t| t.id == *id) {
                Err(PolicyError::TaskAlreadyExists(id.clone()))
            } else {
                Ok(())
            }
        }
        // Decomposing a task into children (parallel fan-out): the parent must
        // exist, and every child id must be fresh (not an existing task, and not
        // duplicated within this decomposition). The parent is the join point.
        PmAction::DecomposeTask { parent, children } => {
            check_pm_authority(who, registry)?;
            if !state.tasks.iter().any(|t| t.id == *parent) {
                return Err(PolicyError::TaskNotFound(parent.clone()));
            }
            let mut seen = std::collections::HashSet::new();
            for c in children {
                if state.tasks.iter().any(|t| t.id == c.id) || !seen.insert(c.id.clone()) {
                    return Err(PolicyError::DuplicateEntity(c.id.clone()));
                }
            }
            Ok(())
        }
        // A hard dependency: both endpoints must exist, be distinct, and the
        // edge must not already exist (deterministic — no dupes).
        PmAction::BlockTaskOn {
            task_id,
            blocking_task_id,
            ..
        } => {
            if !state.tasks.iter().any(|t| t.id == *task_id) {
                return Err(PolicyError::TaskNotFound(task_id.clone()));
            }
            if !state.tasks.iter().any(|t| t.id == *blocking_task_id) {
                return Err(PolicyError::TaskNotFound(blocking_task_id.clone()));
            }
            if task_id == blocking_task_id {
                return Err(PolicyError::TaskNotFound(task_id.clone()));
            }
            if state
                .dependencies
                .iter()
                .any(|d| d.task == *task_id && d.blocking_task == *blocking_task_id)
            {
                return Err(PolicyError::DuplicateEntity(task_id.clone()));
            }
            Ok(())
        }
        // Apply a playbook: PM-only authority, parent task must exist and not
        // be Done, parent must not already have a playbook applied, the
        // packaged playbook must exist in the consultant registry, and the
        // cost band's involvement must satisfy the decision policy.
        PmAction::ApplyPlaybook {
            playbook_id,
            parent_task_id,
            version: _,
            recipe,
        } => {
            check_pm_authority(who, registry)?;
            // Parent must exist
            let parent = state
                .tasks
                .iter()
                .find(|t| t.id == *parent_task_id)
                .ok_or_else(|| PolicyError::TaskNotFound(parent_task_id.clone()))?;
            // Parent must not already be Done
            if parent.status == crate::projection::TaskStatus::Done {
                return Err(PolicyError::TaskStatusError(format!(
                    "cannot apply playbook to task {parent_task_id}: task is already Done"
                )));
            }
            // Parent must not already have a playbook applied
            if parent.playbook_id.is_some() {
                return Err(PolicyError::DuplicateEntity(format!(
                    "task {parent_task_id} already has a playbook applied"
                )));
            }
            // Determine cost band and validate
            let cost_band = if let Some(r) = recipe {
                // Ad-hoc recipe: validate basic structure
                if r.title.is_empty() {
                    return Err(PolicyError::TaskStatusError(
                        "ad-hoc recipe title may not be empty".into(),
                    ));
                }
                if r.steps.is_empty() {
                    return Err(PolicyError::TaskStatusError(
                        "ad-hoc recipe has no steps".into(),
                    ));
                }
                r.cost_band
            } else {
                // Packaged playbook: must exist in consultant registry
                if let Some(reg) = registry {
                    if reg.playbook(playbook_id).is_none() {
                        return Err(PolicyError::PlaybookNotFound(playbook_id.clone()));
                    }
                    // Get cost band from the resolved playbook
                    let (_, pb) = reg.playbook(playbook_id).unwrap();
                    pb.cost_band
                } else {
                    return Err(PolicyError::PlaybookNotFound(playbook_id.clone()));
                }
            };
            // Check cost-band involvement against policy
            use crate::pm::policy::OwnerInvolvement;
            let involvement = match cost_band {
                crate::consultants::playbook::CostBand::Cheap => OwnerInvolvement::Pm,
                crate::consultants::playbook::CostBand::Medium => OwnerInvolvement::Pm,
                crate::consultants::playbook::CostBand::Expensive => OwnerInvolvement::Ask,
            };
            policy::check_proposal(
                match cost_band {
                    crate::consultants::playbook::CostBand::Cheap => {
                        crate::pm::policy::DecisionClass::PlaybookCheap
                    }
                    crate::consultants::playbook::CostBand::Medium => {
                        crate::pm::policy::DecisionClass::PlaybookMedium
                    }
                    crate::consultants::playbook::CostBand::Expensive => {
                        crate::pm::policy::DecisionClass::PlaybookExpensive
                    }
                },
                involvement,
                &state.policy,
            )
        }
        PmAction::AssignTask {
            task_id, assignee, ..
        } => {
            check_pm_authority(who, registry)?;
            let task_exists = state.tasks.iter().any(|t| t.id == *task_id);
            if !task_exists {
                return Err(PolicyError::TaskNotFound(task_id.clone()));
            }
            // The assignee is either a hired agent, the PM (for self-assigned
            // small work via the chat-interface playbook), or the human director
            // (director can take a task on personally and deliver via their harness).
            // Anything else is rejected — and a reserved special role (Advisor)
            // is rejected with a distinct, clearer error.
            if is_advisor_actor(assignee, registry) {
                return Err(PolicyError::SpecialRoleNotAssignable(assignee.clone()));
            }
            if !is_valid_assignee(state, assignee, registry) {
                return Err(PolicyError::AgentNotHired(assignee.clone()));
            }
            Ok(())
        }
        PmAction::StartTask { task_id } => {
            check_assignee(task_id, who, state, registry)?;
            // Fail-closed: a task can only be started from Backlog state.
            // System and director bypass this check (trusted actors).
            let task = state.tasks.iter().find(|t| t.id == *task_id).unwrap();
            if task.status != crate::projection::TaskStatus::Backlog && who != "system" {
                return Err(PolicyError::TaskStatusError(format!(
                    "task {task_id} is not in Backlog state (status={:?})",
                    task.status
                )));
            }
            // Fail-closed isolation (2026-08-12): a task can only be started
            // with an isolated worktree provisioned — unless the assignee is
            // the director (the human works through their own harness, not a
            // Casting worktree) or who is system (trusted seed).
            let task = state.tasks.iter().find(|t| t.id == *task_id).unwrap();
            let assignee = task.assignee.as_deref().unwrap_or("system");
            let needs_worktree = assignee != DIRECTOR && who != "system";
            if needs_worktree
                && !state
                    .worktrees
                    .iter()
                    .any(|w| w.task_id == Some(task_id.clone()))
            {
                return Err(PolicyError::TaskHasNoWorktree(task_id.clone()));
            }
            // Hard-dependency ordering (Blocker Test): a task cannot START
            // while it has unsatisfied hard deps. Fail-closed — the gate, not
            // the PM, enforces ordering so a wrong model can't start early.
            let blockers = state.blocked_by(task_id);
            if !blockers.is_empty() {
                return Err(PolicyError::BlockedByDependency {
                    task_id: task_id.clone(),
                    blockers,
                });
            }
            Ok(())
        }
        // The thin agent git surface: a consultant commits their WIP into their
        // own worktree. They must be the assignee AND their task must have an
        // isolated worktree (isolation is structural, never optional).
        PmAction::CommitToChangeSet { task_id, .. } => {
            check_assignee(task_id, who, state, registry)?;
            if !state
                .worktrees
                .iter()
                .any(|w| w.task_id == Some(task_id.clone()))
            {
                return Err(PolicyError::TaskHasNoWorktree(task_id.clone()));
            }
            Ok(())
        }
        PmAction::ProvisionWorktree { task_id, .. } => {
            check_pm_authority(who, registry)?;
            // Only hired agents get worktrees, plus the PM (who can self-assign
            // via the chat-interface playbook). the director works through their
            // own harness. The task must exist and be assigned to a consultant.
            let task = state
                .tasks
                .iter()
                .find(|t| t.id == *task_id)
                .ok_or_else(|| PolicyError::TaskNotFound(task_id.clone()))?;
            let assignee = task
                .assignee
                .as_deref()
                .ok_or_else(|| PolicyError::TaskUnassigned(task_id.clone()))?;
            if assignee == DIRECTOR {
                return Err(PolicyError::WorktreeForOwner(task_id.clone()));
            }
            if !is_pm_actor(assignee, registry) && !state.agents.iter().any(|a| a.id == assignee) {
                return Err(PolicyError::AgentNotHired(assignee.to_string()));
            }
            // One worktree per task (fail-closed id uniqueness).
            if state
                .worktrees
                .iter()
                .any(|w| w.task_id == Some(task_id.clone()))
            {
                return Err(PolicyError::WorktreeAlreadyProvisioned(task_id.clone()));
            }
            Ok(())
        }
        PmAction::CompleteTask { task_id, .. } => {
            check_assignee(task_id, who, state, registry)?;
            let task = state.tasks.iter().find(|t| t.id == *task_id).unwrap();
            // Fail-closed: a task can only be completed from Working state.
            // System bypasses this check (trusted actor).
            if task.status != crate::projection::TaskStatus::Working && who != "system" {
                return Err(PolicyError::TaskNotInReview(task_id.clone()));
            }
            // Tiered merge gate (2026-08-14): a `pm`-merge task cannot be
            // completed straight to Done by a consultant — it must pass
            // through the PM's review (RequestReview → ReviewTask). `self`-merge
            // tasks, director-delivered tasks, and system tasks may complete
            // directly (the fast path).
            let is_owner_or_system = task.assignee.as_deref() == Some(DIRECTOR) || who == "system";
            if task.merge_authority == crate::types::MergeAuthority::PmMerge && !is_owner_or_system
            {
                return Err(PolicyError::PmMergeRequiresReview(task_id.clone()));
            }
            Ok(())
        }
        PmAction::BlockTask { task_id, .. } => check_assignee(task_id, who, state, registry),
        // Reclassifying merge authority is the escape hatch (scope grew past its
        // assignment label). PM/director/system authority only; the task must exist.
        PmAction::SetMergeAuthority { task_id, .. } => {
            if !(who == DIRECTOR || who == "system" || is_pm_actor(who, registry)) {
                return Err(PolicyError::ActionNotAuthorized(who.to_string()));
            }
            if !state.tasks.iter().any(|t| t.id == *task_id) {
                return Err(PolicyError::TaskNotFound(task_id.clone()));
            }
            Ok(())
        }
        // Submitting work for review: the assignee submits their own work, and
        // the reviewer must be a real agent.
        PmAction::RequestReview { task_id, reviewer } => {
            check_assignee(task_id, who, state, registry)?;
            // Fail-closed: a task can only be sent for review from Working state.
            let task = state.tasks.iter().find(|t| t.id == *task_id).unwrap();
            if task.status != crate::projection::TaskStatus::Working {
                return Err(PolicyError::TaskNotInReview(format!(
                    "task {task_id} is not in Working state (status={:?})",
                    task.status
                )));
            }
            if !state.agents.iter().any(|a| a.id == *reviewer) {
                return Err(PolicyError::AgentNotHired(reviewer.clone()));
            }
            Ok(())
        }
        // Ruling on a review: the task must currently be InReview. Status
        // legality lives in the graph's transition TABLE (the single source of
        // truth) — the gate consults `valid_from_status` rather than hand-writing
        // a status match, so it can never drift from the graph / PM prompt.
        PmAction::ReviewTask { task_id, .. } => {
            let Some(task) = state.tasks.iter().find(|t| t.id == *task_id) else {
                return Err(PolicyError::TaskNotFound(task_id.clone()));
            };
            if !crate::projection::graph::valid_from_status(task.status, "review_task") {
                return Err(PolicyError::TaskNotInReview(task_id.clone()));
            }
            Ok(())
        }
        // Setting a priority is a plan mutation on an existing task.
        PmAction::SetTaskPriority { task_id, .. } => {
            check_pm_authority(who, registry)?;
            let exists = state.tasks.iter().any(|t| t.id == *task_id);
            if exists {
                Ok(())
            } else {
                Err(PolicyError::TaskNotFound(task_id.clone()))
            }
        }
        // Resolving a risk requires it to exist.
        PmAction::ResolveRisk { risk_id, .. } => {
            let exists = state.risks.iter().any(|r| r.id == *risk_id);
            if exists {
                Ok(())
            } else {
                Err(PolicyError::RiskNotFound(risk_id.clone()))
            }
        }
        // A fresh proposal must not under-claim director involvement for its class
        // (the authority-downgrade guard from policy.rs). The claim is checked
        // against the project's EVENT-SOURCED policy (state.policy, folded from
        // DecisionPolicyChanged) — so director-configured autonomy is enforced.
        PmAction::ProposeDecision {
            class,
            involvement,
            subject,
            ..
        } => {
            check_pm_authority(who, registry)?;
            // Reactive anti-thrash: don't accumulate a duplicate OPEN decision
            // on the same subject. The PM must instead supersede a stale one or
            // leave it — never re-propose.
            if let Some(existing) = state.decisions.iter().find(|d| {
                d.status == crate::projection::DecisionStatus::Proposed && d.subject == *subject
            }) {
                return Err(PolicyError::DecisionAlreadyOpen(existing.subject.clone()));
            }
            policy::check_proposal(*class, *involvement, &state.policy)
        }
        // Resolving a decision is the universal decider step; the decision must
        // exist and still be open. The decider (`who`) is whatever the policy
        // routed it to — the actor label is what distinguishes Owner vs agent.
        PmAction::MakeDecision { decision_id, .. } => {
            let Some(dec) = state.decisions.iter().find(|d| d.id == *decision_id) else {
                return Err(PolicyError::DecisionNotFound(decision_id.clone()));
            };
            if dec.status != crate::projection::DecisionStatus::Proposed {
                return Err(PolicyError::DecisionNotOpen(decision_id.clone()));
            }
            // Authority check: the decider must be authorized for this decision
            // class's involvement level (C1). Ask-class decisions require director
            // or system; Pm/Notify/Never can be decided by pm, director, or system.
            use crate::pm::policy::OwnerInvolvement;
            if dec.involvement == OwnerInvolvement::Ask && !matches!(who, "director" | "system") {
                return Err(PolicyError::ActionNotAuthorized(who.to_string()));
            }
            Ok(())
        }
        // Superseding requires the decision exists and isn't already superseded,
        // and the replacing decision must exist.
        PmAction::SupersedeDecision {
            decision_id,
            by_decision_id,
        } => {
            check_pm_authority(who, registry)?;
            if !state.decisions.iter().any(|d| d.id == *decision_id) {
                return Err(PolicyError::DecisionNotFound(decision_id.clone()));
            }
            if !state.decisions.iter().any(|d| d.id == *by_decision_id) {
                return Err(PolicyError::DecisionNotFound(by_decision_id.clone()));
            }
            Ok(())
        }
        // Governance (directives) is director/PM-authority. A plain agent can only
        // raise an Observation (propose); it may not change directive state.
        PmAction::CreateDirective { .. } => check_directive_authority(who),
        PmAction::SuspendDirective { directive_id } => {
            check_directive_authority(who)?;
            check_directive_exists(directive_id, state)
        }
        PmAction::ResumeDirective { directive_id } => {
            check_directive_authority(who)?;
            check_directive_exists(directive_id, state)
        }
        PmAction::ExpireDirective { directive_id } => {
            check_directive_authority(who)?;
            check_directive_exists(directive_id, state)
        }
        PmAction::SupersedeDirective {
            directive_id,
            by_directive_id,
        } => {
            check_directive_authority(who)?;
            check_directive_exists(directive_id, state)?;
            // The replacing directive must land on an existing, ACTIVE target.
            // (A create may reference it via `supersedes`; for the Supersede
            // action `by` must already exist and be active.)
            let Some(by) = state.directives.iter().find(|d| d.id == *by_directive_id) else {
                return Err(PolicyError::DirectiveNotFound(by_directive_id.clone()));
            };
            if by.status != crate::runtime::directive::DirectiveStatus::Active {
                return Err(PolicyError::DirectiveNotFound(by_directive_id.clone()));
            }
            Ok(())
        }
        PmAction::SupersedeOpinion {
            opinion_id,
            by_opinion_id,
        } => {
            // The target must exist and currently be Active (don't re-flip an
            // already-superseded opinion; that'd be a no-op/ambiguity).
            let Some(target) = state.opinions.iter().find(|o| o.id == *opinion_id) else {
                return Err(opinion_not_found(opinion_id));
            };
            if target.status != crate::projection::OpinionStatus::Active {
                return Err(opinion_not_found(opinion_id));
            }
            // The replacing opinion must exist, be distinct, and Active.
            if opinion_id == by_opinion_id {
                return Err(opinion_not_found(by_opinion_id));
            }
            let Some(by) = state.opinions.iter().find(|o| o.id == *by_opinion_id) else {
                return Err(opinion_not_found(by_opinion_id));
            };
            if by.status != crate::projection::OpinionStatus::Active {
                return Err(opinion_not_found(by_opinion_id));
            }
            Ok(())
        }
        // Proposing a governance change needs no directive authority — the PM/
        // agent is PROPOSING, not authoring. It routes to the director (Ask) and
        // is applied only on approval. Encodes the desired change for later.
        PmAction::ProposeDirectiveChange { .. } => Ok(()),
        // Proposing a consultant hire is a proposal, not the hire — the team
        // change happens on director approval (or PM auto-decision per policy).
        // The role must exist in the catalog so a bad role is rejected early.
        PmAction::ProposeConsultant { role_id, .. } => {
            if crate::workspace::role_by_id(role_id).is_none() {
                return Err(PolicyError::UnknownRole(role_id.clone()));
            }
            Ok(())
        }
        // Creating a requirement is idempotency-guarded by id uniqueness.
        PmAction::CreateRequirement { id, .. } => check_unique_entity(
            state.requirements.iter().any(|r| r.id == *id),
            PolicyError::DuplicateEntity(id.clone()),
        ),
        // Raise a risk: id must be fresh.
        PmAction::RaiseRisk { id, .. } => check_unique_entity(
            state.risks.iter().any(|r| r.id == *id),
            PolicyError::DuplicateEntity(id.clone()),
        ),
        // Recording a semantic note / opinion / fact: id must be fresh.
        PmAction::RecordAssumption { id, .. } => check_unique_entity(
            state.assumptions.iter().any(|a| a.id == *id),
            PolicyError::DuplicateEntity(id.clone()),
        ),
        PmAction::RecordConstraint { id, .. } => check_unique_entity(
            state.constraints.iter().any(|c| c.id == *id),
            PolicyError::DuplicateEntity(id.clone()),
        ),
        PmAction::RecordOpinion { id, .. } => check_unique_entity(
            state.opinions.iter().any(|o| o.id == *id),
            PolicyError::DuplicateEntity(id.clone()),
        ),
        PmAction::RecordFact { id, .. } => check_unique_entity(
            state.facts.iter().any(|f| f.id == *id),
            PolicyError::DuplicateEntity(id.clone()),
        ),
        // Importing a briefing / receiving an external request: id must be fresh.
        PmAction::ImportBriefing { id, .. } => check_unique_entity(
            state.briefings.iter().any(|b| b.id == *id),
            PolicyError::DuplicateEntity(id.clone()),
        ),
        PmAction::ReceiveExternalRequest { id, .. } => check_unique_entity(
            state.external_requests.iter().any(|r| r.id == *id),
            PolicyError::DuplicateEntity(id.clone()),
        ),
        // Saving a diagram: id must be fresh.
        PmAction::SaveDiagram { id, .. } => check_unique_entity(
            state.diagrams.iter().any(|d| d.id == *id),
            PolicyError::DuplicateEntity(id.clone()),
        ),
        // Creating an observation: id must be fresh.
        PmAction::CreateObservation { id, .. } => check_unique_entity(
            state.observations.iter().any(|o| o.id == *id),
            PolicyError::DuplicateEntity(id.clone()),
        ),
        // --- Harness guards (2026-08-13) ---
        // Budget + resume are DIRECTOR-only: the circuit breaker sits outside PM
        // control. PauseWork additionally permits the system (liveness
        // watchdog); a plain agent or the PM can never pause/resume work.
        PmAction::SetBudget { limit_usd, .. } => {
            if *limit_usd < 0.0 {
                return Err(PolicyError::GuardAuthority(
                    "budget limit must be >= 0".into(),
                ));
            }
            check_guard_authority(who)
        }
        PmAction::ResumeWork => check_guard_authority(who),
        PmAction::PauseWork { .. } => match who {
            "director" | "system" => Ok(()),
            other => Err(PolicyError::GuardAuthority(other.to_string())),
        },
        // SendMessage and NoOp carry no cross-entity invariant — but they are
        // enumerated EXPLICITLY so any future PmAction variant fails to compile
        // here (fail-closed) rather than silently passing the gate.
        PmAction::SendMessage { .. } => Ok(()),
        PmAction::NoOp => Ok(()),
    }
}

/// Fail-closed id-uniqueness helper: if `exists`, return the given error (a
/// DuplicateEntity); else Ok.
fn check_unique_entity(exists: bool, err: PolicyError) -> Result<(), PolicyError> {
    if exists {
        Err(err)
    } else {
        Ok(())
    }
}

/// PM authority check: only director, pm, or system may perform organizational
/// planning actions (HireAgent, CreateTask, AssignTask, DecomposeTask,
/// ProvisionWorktree, SetTaskPriority, ProposeDecision, SupersedeDecision).
/// Consultants/agents cannot reorganize the project plan — they execute within
/// their assigned tasks (C1).
///
/// "pm" here is resolved by ROLE (the consultant filling
/// `CastRole::ProjectManager`), so the app never hardcodes the PM's id.
fn check_pm_authority(who: &str, registry: Option<&ConsultantRegistry>) -> Result<(), PolicyError> {
    if who == DIRECTOR || who == "system" || is_pm_actor(who, registry) {
        Ok(())
    } else {
        Err(PolicyError::ActionNotAuthorized(who.to_string()))
    }
}

/// Governance is DIRECTOR-only authority: only the director may create or change
/// directives. The PM/system and plain agents cannot mutate governance —
/// governance is the project's constitution, too important to delegate. Any
/// non-director actor is rejected (they may still *propose* via an Observation).
fn check_directive_authority(who: &str) -> Result<(), PolicyError> {
    match who {
        "director" => Ok(()),
        other => Err(PolicyError::DirectiveAuthority(other.to_string())),
    }
}

/// Guard control (budget set / resume) is DIRECTOR-only — the hard rails sit
/// OUTSIDE the PM's control (the PM can be confused, compromised, or just
/// wrong). PauseWork is handled inline (director OR the system watchdog).
fn check_guard_authority(who: &str) -> Result<(), PolicyError> {
    match who {
        "director" => Ok(()),
        other => Err(PolicyError::GuardAuthority(other.to_string())),
    }
}

fn check_directive_exists(directive_id: &str, state: &Projection) -> Result<(), PolicyError> {
    if state.directives.iter().any(|d| d.id == directive_id) {
        Ok(())
    } else {
        Err(PolicyError::DirectiveNotFound(directive_id.to_string()))
    }
}

/// The referenced opinion isn't a valid supersede target (absent or not
/// currently Active).
fn opinion_not_found(opinion_id: &str) -> PolicyError {
    PolicyError::OpinionNotFound(opinion_id.to_string())
}

/// For Start/Complete/Block: the task must exist, have an assignee, and the
/// actor must BE that assignee. `system` may always act (it seeds tasks).
/// The PM may also act on tasks assigned to the director — the director is a human
/// without an agent loop, so the PM acts as their proxy for lifecycle
/// operations (start/complete/block).
fn check_assignee(
    task_id: &str,
    who: &str,
    state: &Projection,
    registry: Option<&ConsultantRegistry>,
) -> Result<(), PolicyError> {
    let Some(task) = state.tasks.iter().find(|t| t.id == task_id) else {
        return Err(PolicyError::TaskNotFound(task_id.to_string()));
    };
    // `system` is trusted: it seeds initial state and does not bypass a real
    // assignee in practice. This keeps the scripted onboard plan working.
    if who == "system" {
        return Ok(());
    }
    let Some(assignee) = &task.assignee else {
        return Err(PolicyError::TaskUnassigned(task_id.to_string()));
    };
    // The PM acts as proxy for the director (human has no agent loop).
    if is_pm_actor(who, registry) && assignee == "director" {
        return Ok(());
    }
    if who != assignee {
        return Err(PolicyError::NotAssignee {
            task_id: task_id.to_string(),
            actor: who.to_string(),
            assignee: assignee.clone(),
        });
    }
    Ok(())
}
