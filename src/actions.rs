//! The PM's structured action vocabulary + policy gate.
//!
//! This is the seam between *reasoning* and *execution* (docs/ADDENDUM.md §16):
//!
//! ```text
//! reasoning → structured proposed actions → policy validation → execution → domain events
//! ```
//!
//! Today the reasoning is a deterministic scripted policy in `pm.rs`. Tomorrow
//! it will be an LLM client. Both produce the SAME typed `PmAction`s, which are
//! validated by the pure policy gate here before anything touches the event
//! store. That gate is what stops a wrong model (or a wrong script) from
//! burning tokens or corrupting project state: an action that violates project
//! invariants (assigning an unhired agent, completing a nonexistent task) is
//! rejected before any event is appended.
//!
//! The policy gate is deliberately a *pure* function of the action and the
//! current projection — no I/O — so it is trivially unit-testable and safe to
//! run in front of an arbitrary untrusted producer.

use crate::event::{Actor, Aggregate, Event, EventType, Metadata};
use crate::policy::{self, DecisionClass, OwnerInvolvement};
use crate::projection::Projection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One organizational move the PM may propose. Serde-tagged so an LLM can emit
/// it as JSON and it round-trips 1:1 with what a scripted policy builds.
///
/// Each variant carries exactly the fields needed to execute the action; the
/// aggregate id of the resulting event is the action's entity id. Several map
/// to a single domain event; a few span two (e.g. proposing a decision also
/// results in a message to the owner).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PmAction {
    /// Bring a consultant into the company.
    HireAgent {
        agent_id: String,
        role: String,
    },
    /// Record a requirement derived from owner intent.
    CreateRequirement {
        id: String,
        title: String,
        description: String,
    },
    /// Place a task on the board.
    CreateTask {
        id: String,
        title: String,
        kind: String,
    },
    /// Assign a task to a hired agent.
    AssignTask {
        task_id: String,
        assignee: String,
    },
    StartTask {
        task_id: String,
    },
    CompleteTask {
        task_id: String,
        result: String,
    },
    BlockTask {
        task_id: String,
        reason: String,
    },
    /// Change a task's priority (a plan mutation; reduces to TaskPriorityChanged).
    SetTaskPriority {
        task_id: String,
        priority: crate::plan::Priority,
    },
    /// Raise a first-class risk (semantic object, SEMANTIC_EVENTS §8).
    RaiseRisk {
        id: String,
        subject: String,
        severity: String,
    },
    /// Resolve (or mark materialized) a risk.
    ResolveRisk {
        risk_id: String,
        status: crate::projection::RiskStatus,
    },
    /// Record a project assumption (semantic note).
    RecordAssumption {
        id: String,
        body: String,
    },
    /// Record a project constraint (semantic note).
    RecordConstraint {
        id: String,
        body: String,
    },
    /// Create a governance directive (docs/INTENT.md). Owner/PM-authority only.
    CreateDirective {
        id: String,
        kind: crate::directive::DirectiveKind,
        statement: String,
        scope: Vec<String>,
        strength: crate::directive::DirectiveStrength,
        supersedes: Option<String>,
    },
    /// Suspend a directive (stop it governing) — authority-gated.
    SuspendDirective {
        directive_id: String,
    },
    /// Resume a suspended directive — authority-gated.
    ResumeDirective {
        directive_id: String,
    },
    /// Replace a directive with another (supersession) — authority-gated.
    SupersedeDirective {
        directive_id: String,
        by_directive_id: String,
    },
    /// Expire a directive — authority-gated.
    ExpireDirective {
        directive_id: String,
    },
    /// Propose a change to governance (docs/INTENT.md). The PM/agents cannot
    /// author directives directly, but they CAN propose a GovernanceChange
    /// decision, which routes to the owner (Ask); on approval the change is
    /// applied on the owner's authority.
    ProposeDirectiveChange {
        id: String,
        subject: String,
        kind: crate::directive::DirectiveKind,
        statement: String,
        scope: Vec<String>,
        strength: crate::directive::DirectiveStrength,
        supersedes: Option<String>,
    },
    /// An agent raises a noticed observation (the feedback loop).
    CreateObservation {
        id: String,
        severity: String,
        subject: String,
        body: String,
        pm_action_required: bool,
    },
    /// Ask the owner to rule on a decision (delegated authority).
    ProposeDecision {
        id: String,
        subject: String,
        options: Value,
        recommendation: String,
        /// The decision's class — drives which owner involvement the policy
        /// engine requires (and thus who the decision-maker is).
        class: DecisionClass,
        /// The resolved owner involvement claimed by the producer. `validate`
        /// rejects this if it undercuts what the policy requires for `class`
        /// (authority-downgrade guard).
        involvement: OwnerInvolvement,
    },
    /// Resolve a decision. The universal decision-maker step: the actor is who
    /// decided — `Owner` after being asked, or a delegated PM/agent (per policy).
    /// Produces `DecisionMade`; there is no separate owner-decision event.
    MakeDecision {
        decision_id: String,
        approved: bool,
        note: Option<String>,
    },
    /// Supersede a decision with a newer one (history preserved, never deleted).
    /// Status -> Superseded; `by_decision_id` links the replacement.
    SupersedeDecision {
        decision_id: String,
        by_decision_id: String,
    },
    /// A human-readable message to the owner / another agent.
    SendMessage {
        to: String,
        body: String,
    },
    /// Explicitly conclude "nothing to do" (anti-thrash).
    NoOp,
}

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
    /// A plain agent (not owner/PM/system) trying to change governance.
    DirectiveAuthority(String),
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
            PolicyError::DirectiveAuthority(who) => {
                write!(
                    f,
                    "{who} lacks authority to change project governance (directives)"
                )
            }
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
            let assignee_hired = state.agents.iter().any(|a| a.id == *assignee);
            if !assignee_hired {
                return Err(PolicyError::AgentNotHired(assignee.clone()));
            }
            Ok(())
        }
        PmAction::StartTask { task_id } => check_assignee(task_id, who, state),
        PmAction::CompleteTask { task_id, .. } => check_assignee(task_id, who, state),
        PmAction::BlockTask { task_id, .. } => check_assignee(task_id, who, state),
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
        // Proposing a governance change needs no directive authority — the PM/
        // agent is PROPOSING, not authoring. It routes to the owner (Ask) and
        // is applied only on approval. Encodes the desired change for later.
        PmAction::ProposeDirectiveChange { .. } => Ok(()),
        // Hire-less, idempotency-neutral or read-only actions pass through;
        // NoOp, CreateRequirement, CreateObservation, and SendMessage carry no
        // cross-entity invariant to check at this layer.
        _ => Ok(()),
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

impl PmAction {
    /// Map an action, performed by `who`, into the ordered domain events that
    /// record it — with provenance (`causation`/`correlation`/`agent_run_id`)
    /// already attached. Returns zero events for `NoOp`.
    pub fn to_events(
        &self,
        project: &str,
        who: &str,
        cause: &Event,
        correlation: &str,
    ) -> Vec<Event> {
        let actor = actor_for(who);
        let meta = linked(cause, correlation);
        match self {
            PmAction::HireAgent { agent_id, role } => vec![ev(
                project,
                actor,
                agent_id,
                "agent",
                EventType::AgentHired,
                json!({ "role": role }),
                meta,
            )],
            PmAction::CreateRequirement {
                id,
                title,
                description,
            } => vec![ev(
                project,
                actor,
                id,
                "requirement",
                EventType::RequirementCreated,
                json!({ "title": title, "description": description }),
                meta,
            )],
            PmAction::CreateTask { id, title, kind } => vec![ev(
                project,
                actor,
                id,
                "task",
                EventType::TaskCreated,
                json!({ "title": title, "kind": kind }),
                meta,
            )],
            PmAction::AssignTask { task_id, assignee } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskAssigned,
                json!({ "assignee": assignee }),
                meta,
            )],
            PmAction::StartTask { task_id } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskStarted,
                json!({}),
                meta,
            )],
            PmAction::CompleteTask { task_id, result } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskCompleted,
                json!({ "result": result }),
                meta,
            )],
            PmAction::BlockTask { task_id, reason } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskBlocked,
                json!({ "reason": reason }),
                meta,
            )],
            PmAction::SetTaskPriority { task_id, priority } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskPriorityChanged,
                // `from` is omitted here (to_events lacks the projection); the
                // reducer only needs `to` for current state.
                json!({ "task_id": task_id, "to": priority }),
                meta,
            )],
            PmAction::RaiseRisk {
                id,
                subject,
                severity,
            } => vec![ev(
                project,
                actor,
                id,
                "risk",
                EventType::RiskRaised,
                json!({ "subject": subject, "severity": severity }),
                meta,
            )],
            PmAction::ResolveRisk { risk_id, status } => vec![ev(
                project,
                actor,
                risk_id,
                "risk",
                EventType::RiskUpdated,
                json!({ "status": status }),
                meta,
            )],
            PmAction::RecordAssumption { id, body } => vec![ev(
                project,
                actor,
                id,
                "assumption",
                EventType::AssumptionRecorded,
                json!({ "body": body }),
                meta,
            )],
            PmAction::RecordConstraint { id, body } => vec![ev(
                project,
                actor,
                id,
                "constraint",
                EventType::ConstraintRecorded,
                json!({ "body": body }),
                meta,
            )],
            PmAction::CreateDirective {
                id,
                kind,
                statement,
                scope,
                strength,
                supersedes,
            } => vec![ev(
                project,
                actor,
                id,
                "directive",
                EventType::ProjectDirectiveCreated,
                json!({
                    "kind": kind,
                    "statement": statement,
                    "scope": scope,
                    "strength": strength,
                    "created_by": null, // derived from the event actor by the reducer
                    "supersedes": supersedes,
                }),
                meta,
            )],
            PmAction::SuspendDirective { directive_id } => vec![ev(
                project,
                actor,
                directive_id,
                "directive",
                EventType::ProjectDirectiveSuspended,
                json!({}),
                meta,
            )],
            PmAction::ResumeDirective { directive_id } => vec![ev(
                project,
                actor,
                directive_id,
                "directive",
                EventType::ProjectDirectiveResumed,
                json!({}),
                meta,
            )],
            PmAction::ExpireDirective { directive_id } => vec![ev(
                project,
                actor,
                directive_id,
                "directive",
                EventType::ProjectDirectiveExpired,
                json!({}),
                meta,
            )],
            PmAction::SupersedeDirective {
                directive_id,
                by_directive_id,
            } => vec![ev(
                project,
                actor,
                directive_id,
                "directive",
                EventType::ProjectDirectiveSuperseded,
                json!({ "superseded_by": by_directive_id }),
                meta,
            )],
            PmAction::CreateObservation {
                id,
                severity,
                subject,
                body,
                pm_action_required,
            } => vec![ev(
                project,
                actor,
                id,
                "observation",
                EventType::ObservationCreated,
                json!({
                    "severity": severity,
                    "subject": subject,
                    "body": body,
                    "pm_action_required": pm_action_required,
                }),
                meta,
            )],
            PmAction::ProposeDecision {
                id,
                subject,
                options,
                recommendation,
                class,
                involvement,
            } => vec![ev(
                project,
                actor,
                id,
                "decision",
                EventType::DecisionProposed,
                json!({
                    "subject": subject,
                    "options": options,
                    "recommendation": recommendation,
                    "class": class,
                    "involvement": involvement,
                }),
                meta,
            )],
            PmAction::ProposeDirectiveChange {
                id,
                subject,
                kind,
                statement,
                scope,
                strength,
                supersedes,
            } => vec![ev(
                project,
                actor,
                id,
                "decision",
                EventType::DecisionProposed,
                json!({
                    "subject": subject,
                    "options": serde_json::json!({
                        "governance_change": {
                            "kind": kind,
                            "statement": statement,
                            "scope": scope,
                            "strength": strength,
                            "supersedes": supersedes,
                        },
                    }),
                    "recommendation": format!("Approve governance change: {subject}"),
                    "class": crate::policy::DecisionClass::GovernanceChange,
                    "involvement": crate::policy::OwnerInvolvement::Ask,
                }),
                meta,
            )],
            PmAction::MakeDecision {
                decision_id,
                approved,
                note,
            } => vec![ev(
                project,
                actor,
                decision_id,
                "decision",
                EventType::DecisionMade,
                json!({
                    "approved": approved,
                    "note": note,
                }),
                meta,
            )],
            PmAction::SupersedeDecision {
                decision_id,
                by_decision_id,
            } => vec![ev(
                project,
                actor,
                decision_id,
                "decision",
                EventType::DecisionSuperseded,
                json!({ "superseded_by": by_decision_id }),
                meta,
            )],
            PmAction::SendMessage { to, body } => vec![ev(
                project,
                actor,
                &format!("msg-{}", cause.sequence),
                "message",
                EventType::MessageSent,
                json!({ "to": to, "body": body }),
                meta,
            )],
            PmAction::NoOp => vec![],
        }
    }
}

/// Convert a `who` label to the typed actor. `"system"` and `"owner"` map to
/// their domain actors; anything else is an agent id.
pub fn actor_for(who: &str) -> Actor {
    match who {
        "system" => Actor::System,
        "owner" => Actor::Owner,
        id => Actor::Agent { id: id.to_string() },
    }
}

/// Build metadata linking the new events to the owner event that caused them —
/// the "why?" provenance chain (brief §11, addendum §24).
fn linked(causation: &Event, correlation: &str) -> Metadata {
    Metadata {
        correlation_id: Some(correlation.to_string()),
        causation_id: Some(causation.event_id),
        agent_run_id: Some(format!("sim-run-{}", causation.sequence)),
    }
}

/// Make a domain event with provenance already attached.
fn ev(
    project: &str,
    actor: Actor,
    id: &str,
    kind: &str,
    event_type: EventType,
    data: Value,
    meta: Metadata,
) -> Event {
    let mut e = Event::new(
        project,
        actor,
        event_type,
        Aggregate {
            kind: kind.to_string(),
            id: id.to_string(),
        },
        data,
    );
    e.metadata = meta;
    e
}

/// Build the owner-authored `DecisionMade` event (review finding C: centralize
/// event shapes here so web.rs and to_events never drift). The owner resolves a
/// proposed decision; `subject` is the decision's subject for the owner's audit
/// trail, and `note` the verdict's rationale.
pub fn owner_decision_made(
    project: &str,
    decision_id: &str,
    subject: &str,
    approved: bool,
    note: Option<String>,
) -> Event {
    Event::new(
        project,
        Actor::Owner,
        EventType::DecisionMade,
        Aggregate {
            kind: "decision".into(),
            id: decision_id.into(),
        },
        json!({
            "subject": subject,
            "approved": approved,
            "note": note.unwrap_or_default(),
        }),
    )
}

/// Build the owner-authored `DecisionPolicyChanged` event (owner configures the
/// owner-involvement required for a decision class).
pub fn owner_policy_changed(
    project: &str,
    class: crate::policy::DecisionClass,
    involvement: crate::policy::OwnerInvolvement,
) -> Event {
    Event::new(
        project,
        Actor::Owner,
        EventType::DecisionPolicyChanged,
        Aggregate {
            kind: "decision_policy".into(),
            id: format!("{class:?}"),
        },
        json!({
            "class": class,
            "involvement": involvement,
        }),
    )
}

/// Build the owner-authored `ProjectDirectiveCreated` event (owner sets
/// governance). `created_by` is hardcoded to the owner because a directive,
/// once created, is attributed to its author regardless of later security.
pub fn owner_directive_created(
    project: &str,
    id: &str,
    kind: crate::directive::DirectiveKind,
    statement: &str,
    scope: Vec<String>,
    strength: crate::directive::DirectiveStrength,
) -> Event {
    Event::new(
        project,
        Actor::Owner,
        EventType::ProjectDirectiveCreated,
        Aggregate {
            kind: "directive".into(),
            id: id.into(),
        },
        json!({
            "kind": kind,
            "statement": statement,
            "scope": scope,
            "strength": strength,
            "created_by": "owner",
            "supersedes": null,
        }),
    )
}
