//! The policy gate: validation of a proposed action against the projection, and
//! the `PolicyError` rejection vocabulary.
use super::action::{is_valid_assignee, PmAction, OWNER};
use crate::policy;
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
    /// Reviewing a task that isn't currently in review.
    TaskNotInReview(String),
    /// Starting/completing/blocking a task that has not been assigned yet.
    TaskUnassigned(String),
    /// Starting/completing/blocking a task by someone other than its assignee.
    NotAssignee {
        task_id: String,
        actor: String,
        assignee: String,
    },
    /// The authority-downgrade guard fired: a decision was proposed with less
    /// owner involvement than its class's policy requires (from `policy.rs`).
    DecisionPolicy(policy::PolicyError),
    /// Making a decision (resolving it) on one that does not exist.
    DecisionNotFound(String),
    /// Making a decision on one not yet proposed / already decided.
    DecisionNotOpen(String),
    /// Resolving/updating a risk that does not exist.
    RiskNotFound(String),
    /// Acting on a directive that does not exist.
    DirectiveNotFound(String),
    /// A referenced opinion doesn't exist (or isn't supersede-able).
    OpinionNotFound(String),
    /// A plain agent (not owner/PM/system) trying to change governance.
    DirectiveAuthority(String),
    /// Hiring/proposing a role that isn't in the catalog.
    UnknownRole(String),
    /// Creating an entity whose id already exists (fail-closed id uniqueness for
    /// all create actions, not just tasks/agents).
    DuplicateEntity(String),
    /// Starting a task that has no provisioned worktree (fail-closed isolation:
    /// a consultant cannot work un-isolated — the platform provisions the
    /// workspace at summon). "Task X has no isolated worktree".
    TaskHasNoWorktree(String),
    /// Provisioning a worktree for a task that already has one.
    WorktreeAlreadyProvisioned(String),
    /// Provisioning a worktree for a task assigned to the owner (the human
    /// works through their own harness, not a Casting worktree).
    WorktreeForOwner(String),
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
            PolicyError::TaskNotInReview(id) => {
                write!(f, "cannot review task {id}: it is not in review")
            }
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
            PolicyError::DecisionPolicy(e) => write!(f, "{e}"),
            PolicyError::DecisionNotFound(id) => {
                write!(f, "cannot resolve decision {id}: no such decision")
            }
            PolicyError::DecisionNotOpen(id) => {
                write!(
                    f,
                    "cannot resolve decision {id}: not open (proposed, unresolved)"
                )
            }
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
            PolicyError::TaskHasNoWorktree(id) => write!(
                f,
                "cannot start task {id}: no isolated worktree provisioned (the platform provisions it at summon)"
            ),
            PolicyError::WorktreeAlreadyProvisioned(id) => {
                write!(f, "cannot provision worktree for task {id}: one already exists")
            }
            PolicyError::WorktreeForOwner(id) => write!(
                f,
                "cannot provision worktree for task {id}: assigned to the owner (the human works through their own harness)"
            ),
        }
    }
}

impl std::error::Error for PolicyError {}

/// Validate one action, performed by `who`, against the current projection.
/// Pure and infallible on the store — returns `Ok(())` when the action may
/// proceed.
///
/// `who` is the label from a `PlannedAction` ("system", "owner", or an agent
/// id). StartTask/CompleteTask/BlockTask additionally require that `who` IS
/// the task's assignee — the gate stops the wrong agent (or an LLM mistake)
/// from mutating someone else's task.
pub fn validate(action: &PmAction, who: &str, state: &Projection) -> Result<(), PolicyError> {
    match action {
        PmAction::HireAgent { agent_id, .. } => {
            if state.agents.iter().any(|a| a.id == *agent_id) {
                Err(PolicyError::AgentAlreadyHired(agent_id.clone()))
            } else {
                Ok(())
            }
        }
        PmAction::CreateTask { id, .. } => {
            if state.tasks.iter().any(|t| t.id == *id) {
                Err(PolicyError::TaskAlreadyExists(id.clone()))
            } else {
                Ok(())
            }
        }
        PmAction::AssignTask {
            task_id, assignee, ..
        } => {
            let task_exists = state.tasks.iter().any(|t| t.id == *task_id);
            if !task_exists {
                return Err(PolicyError::TaskNotFound(task_id.clone()));
            }
            // The assignee is either a hired agent OR the human owner (owner can
            // take a task on personally and deliver via their harness). Anything
            // else is rejected.
            if !is_valid_assignee(state, assignee) {
                return Err(PolicyError::AgentNotHired(assignee.clone()));
            }
            Ok(())
        }
        PmAction::StartTask { task_id } => {
            check_assignee(task_id, who, state)?;
            // Fail-closed isolation (2026-08-12): a task can only be started
            // with an isolated worktree provisioned — unless the assignee is
            // the owner (the human works through their own harness, not a
            // Casting worktree) or who is system (trusted seed).
            let task = state.tasks.iter().find(|t| t.id == *task_id).unwrap();
            let assignee = task.assignee.as_deref().unwrap_or("system");
            let needs_worktree = assignee != OWNER && who != "system";
            if needs_worktree && !state.worktrees.iter().any(|w| w.task_id == *task_id) {
                return Err(PolicyError::TaskHasNoWorktree(task_id.clone()));
            }
            Ok(())
        }
        PmAction::ProvisionWorktree { task_id, .. } => {
            // Only hired agents get worktrees; the owner works through their
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
            if assignee == OWNER {
                return Err(PolicyError::WorktreeForOwner(task_id.clone()));
            }
            if !state.agents.iter().any(|a| a.id == assignee) {
                return Err(PolicyError::AgentNotHired(assignee.to_string()));
            }
            // One worktree per task (fail-closed id uniqueness).
            if state.worktrees.iter().any(|w| w.task_id == *task_id) {
                return Err(PolicyError::WorktreeAlreadyProvisioned(task_id.clone()));
            }
            Ok(())
        }
        PmAction::CompleteTask { task_id, .. } => check_assignee(task_id, who, state),
        PmAction::BlockTask { task_id, .. } => check_assignee(task_id, who, state),
        // Submitting work for review: the assignee submits their own work, and
        // the reviewer must be a real agent.
        PmAction::RequestReview { task_id, reviewer } => {
            check_assignee(task_id, who, state)?;
            if !state.agents.iter().any(|a| a.id == *reviewer) {
                return Err(PolicyError::AgentNotHired(reviewer.clone()));
            }
            Ok(())
        }
        // Ruling on a review: the task must be currently InReview.
        PmAction::ReviewTask { task_id, .. } => {
            let Some(task) = state.tasks.iter().find(|t| t.id == *task_id) else {
                return Err(PolicyError::TaskNotFound(task_id.clone()));
            };
            if task.status != crate::projection::TaskStatus::InReview {
                return Err(PolicyError::TaskNotInReview(task_id.clone()));
            }
            Ok(())
        }
        // Setting a priority is a plan mutation on an existing task.
        PmAction::SetTaskPriority { task_id, .. } => {
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
        // A fresh proposal must not under-claim owner involvement for its class
        // (the authority-downgrade guard from policy.rs). The claim is checked
        // against the project's EVENT-SOURCED policy (state.policy, folded from
        // DecisionPolicyChanged) — so owner-configured autonomy is enforced.
        PmAction::ProposeDecision {
            class, involvement, ..
        } => policy::check_proposal(*class, *involvement, &state.policy)
            .map_err(PolicyError::DecisionPolicy),
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
            Ok(())
        }
        // Superseding requires the decision exists and isn't already superseded,
        // and the replacing decision must exist.
        PmAction::SupersedeDecision {
            decision_id,
            by_decision_id,
        } => {
            if !state.decisions.iter().any(|d| d.id == *decision_id) {
                return Err(PolicyError::DecisionNotFound(decision_id.clone()));
            }
            if !state.decisions.iter().any(|d| d.id == *by_decision_id) {
                return Err(PolicyError::DecisionNotFound(by_decision_id.clone()));
            }
            Ok(())
        }
        // Governance (directives) is owner/PM-authority. A plain agent can only
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
            if by.status != crate::directive::DirectiveStatus::Active {
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
        // agent is PROPOSING, not authoring. It routes to the owner (Ask) and
        // is applied only on approval. Encodes the desired change for later.
        PmAction::ProposeDirectiveChange { .. } => Ok(()),
        // Proposing a consultant hire is a proposal, not the hire — the team
        // change happens on owner approval (or PM auto-decision per policy).
        // The role must exist in the catalog so a bad role is rejected early.
        PmAction::ProposeConsultant { role_id, .. } => {
            if crate::cast::role_by_id(role_id).is_none() {
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

/// Governance is OWNER-only authority: only the owner may create or change
/// directives. The PM/system and plain agents cannot mutate governance —
/// governance is the project's constitution, too important to delegate. Any
/// non-owner actor is rejected (they may still *propose* via an Observation).
fn check_directive_authority(who: &str) -> Result<(), PolicyError> {
    match who {
        "owner" => Ok(()),
        other => Err(PolicyError::DirectiveAuthority(other.to_string())),
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
fn check_assignee(task_id: &str, who: &str, state: &Projection) -> Result<(), PolicyError> {
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
    if who != assignee {
        return Err(PolicyError::NotAssignee {
            task_id: task_id.to_string(),
            actor: who.to_string(),
            assignee: assignee.clone(),
        });
    }
    Ok(())
}
