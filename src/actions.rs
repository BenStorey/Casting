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
        owner_involvement: String,
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
            PolicyError::AgentNotHired(id) => write!(f, "cannot assign task to {id}: not hired"),
        }
    }
}

impl std::error::Error for PolicyError {}

/// Validate one action against the current projection. Pure and infallible on
/// the store — returns `Ok(())` when the action may proceed.
pub fn validate(action: &PmAction, state: &Projection) -> Result<(), PolicyError> {
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
        PmAction::StartTask { task_id } => task_exists(task_id, state),
        PmAction::CompleteTask { task_id, .. } => task_exists(task_id, state),
        PmAction::BlockTask { task_id, .. } => task_exists(task_id, state),
        // Hires-less, idempotency-neutral or read-only actions pass through;
        // NoOp, CreateRequirement, CreateObservation, ProposeDecision and
        // SendMessage carry no cross-entity invariant to check at this layer.
        _ => Ok(()),
    }
}

fn task_exists(task_id: &str, state: &Projection) -> Result<(), PolicyError> {
    if state.tasks.iter().any(|t| t.id == task_id) {
        Ok(())
    } else {
        Err(PolicyError::TaskNotFound(task_id.to_string()))
    }
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
                owner_involvement,
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
                    "owner_involvement": owner_involvement,
                }),
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
