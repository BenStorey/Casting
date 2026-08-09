//! Current-state projections, derived from the append-only event log.
//!
//! Per docs/CASTING_PROJECT_BRIEF.md §9/§14 and the handoff §4: event history is
//! the source of truth; these projections are recomputable, queryable current
//! state and are NEVER authoritative or stored-and-drifting. We rebuild them on
//! demand by folding the log (idempotent and fine for slice one).

use crate::event::{Event, EventType};
use crate::store::EventStore;
use anyhow::Result;
use serde::Serialize;

/// A hired consultant/agent.
#[derive(Debug, Clone, Serialize)]
pub struct Agent {
    pub id: String,
    pub role: String,
}

/// A product requirement the owner / PM agreed on.
#[derive(Debug, Clone, Serialize)]
pub struct Requirement {
    pub id: String,
    pub title: String,
    pub description: String,
}

/// Where a task sits on the board (a projection of its lifecycle events).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TaskStatus {
    #[serde(rename = "backlog")]
    Backlog,
    #[serde(rename = "working")]
    Working,
    #[serde(rename = "blocked")]
    Blocked,
    #[serde(rename = "done")]
    Done,
}

/// A task. Status is derived from TaskCreated->TaskAssigned->TaskStarted->...->TaskCompleted.
#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: TaskStatus,
    pub assignee: Option<String>,
}

/// A recordable decision and its eventual owner verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DecisionStatus {
    #[serde(rename = "proposed")]
    Proposed,
    #[serde(rename = "approved")]
    Approved,
    #[serde(rename = "rejected")]
    Rejected,
}

#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub id: String,
    pub subject: String,
    pub options: serde_json::Value,
    pub recommendation: Option<String>,
    pub status: DecisionStatus,
    pub owner_verdict: Option<String>,
}

/// Human-readable message (owner <-> PM, agents). Structured, not real email.
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    pub body: String,
}

/// Something an agent noticed.
#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub id: String,
    pub from: String,
    pub severity: String,
    pub subject: String,
    pub body: String,
}

/// A branch in the artifact repo (semantic Git event, ADDENDUM §20/§23).
#[derive(Debug, Clone, Serialize)]
pub struct Branch {
    pub name: String,
    /// The task this branch is associated with, if known (ADDENDUM §20).
    pub task_id: Option<String>,
}

/// A commit observed on a branch (semantic Git event, ADDENDUM §23).
/// Git remains authoritative for the commit; Casting owns the organizational
/// association (ADDENDUM §24).
#[derive(Debug, Clone, Serialize)]
pub struct Commit {
    pub sha: String,
    /// The branch this commit was observed on.
    pub branch: String,
    /// The commit message subject (first line).
    pub message: String,
    /// The author of the commit (git author name).
    pub author: String,
    /// The task this commit is associated with, if known.
    pub task_id: Option<String>,
}

/// A completed merge (semantic Git event, ADDENDUM §23).
#[derive(Debug, Clone, Serialize)]
pub struct Merge {
    /// The merge commit sha.
    pub sha: String,
    /// The branch that was merged.
    pub from_branch: String,
    /// The branch that received the merge.
    pub to_branch: String,
}

/// The full current-state projection for a project.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Projection {
    pub project_id: String,
    pub agents: Vec<Agent>,
    pub requirements: Vec<Requirement>,
    pub tasks: Vec<Task>,
    pub decisions: Vec<Decision>,
    pub messages: Vec<Message>,
    pub observations: Vec<Observation>,
    /// Branches in the artifact repo (semantic Git events).
    pub branches: Vec<Branch>,
    /// Commits observed on branches (semantic Git events).
    pub commits: Vec<Commit>,
    /// Completed merges (semantic Git events).
    pub merges: Vec<Merge>,
}

impl Projection {
    /// Fold the whole event log for `project_id` into current state.
    /// Recomputable and idempotent; called per-request (slice one).
    pub fn build<S: EventStore>(store: &S, project_id: &str) -> Result<Self> {
        let events = store.read_since(project_id, 0)?;
        let mut p = Projection {
            project_id: project_id.to_string(),
            ..Default::default()
        };
        for e in &events {
            p.apply(e);
        }
        Ok(p)
    }

    /// Apply a single event to the running projection.
    pub(crate) fn apply(&mut self, e: &Event) {
        match e.event_type {
            EventType::ProjectCreated => {}
            EventType::AgentHired => self.agents.push(Agent {
                id: e.aggregate.id.clone(),
                role: string_field(e, "role").unwrap_or_default(),
            }),
            EventType::RequirementCreated => self.requirements.push(Requirement {
                id: e.aggregate.id.clone(),
                title: string_field(e, "title").unwrap_or_default(),
                description: string_field(e, "description").unwrap_or_default(),
            }),
            EventType::RequirementChanged => {
                if let Some(req) = self
                    .requirements
                    .iter_mut()
                    .find(|r| r.id == e.aggregate.id)
                {
                    if let Some(desc) = string_field(e, "description") {
                        req.description = desc;
                    }
                }
            }
            EventType::TaskCreated => self.tasks.push(Task {
                id: e.aggregate.id.clone(),
                title: string_field(e, "title").unwrap_or_default(),
                kind: string_field(e, "kind").unwrap_or_else(|| "feature".into()),
                status: TaskStatus::Backlog,
                assignee: None,
            }),
            EventType::TaskAssigned => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.id == e.aggregate.id) {
                    task.assignee = string_field(e, "assignee");
                }
            }
            EventType::TaskStarted => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.id == e.aggregate.id) {
                    task.status = TaskStatus::Working;
                }
            }
            EventType::TaskBlocked => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.id == e.aggregate.id) {
                    task.status = TaskStatus::Blocked;
                }
            }
            EventType::TaskCompleted => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.id == e.aggregate.id) {
                    task.status = TaskStatus::Done;
                }
            }
            EventType::ObservationCreated => self.observations.push(Observation {
                id: e.aggregate.id.clone(),
                from: actor_name(e),
                severity: string_field(e, "severity").unwrap_or_else(|| "info".into()),
                subject: string_field(e, "subject").unwrap_or_default(),
                body: string_field(e, "body").unwrap_or_default(),
            }),
            EventType::DecisionProposed => self.decisions.push(Decision {
                id: e.aggregate.id.clone(),
                subject: string_field(e, "subject").unwrap_or_default(),
                options: e
                    .data
                    .get("options")
                    .cloned()
                    .unwrap_or(serde_json::json!({})),
                recommendation: string_field(e, "recommendation"),
                status: DecisionStatus::Proposed,
                owner_verdict: None,
            }),
            EventType::OwnerDecisionRecorded => {
                // The aggregate id is the decision being ruled on.
                if let Some(dec) = self.decisions.iter_mut().find(|d| d.id == e.aggregate.id) {
                    let approved = bool_field(e, "approved").unwrap_or(false);
                    dec.status = if approved {
                        DecisionStatus::Approved
                    } else {
                        DecisionStatus::Rejected
                    };
                    dec.owner_verdict = string_field(e, "note");
                }
            }
            EventType::MessageSent => self.messages.push(Message {
                id: e.aggregate.id.clone(),
                from: actor_name(e),
                to: string_field(e, "to").unwrap_or_else(|| "owner".into()),
                body: string_field(e, "body").unwrap_or_default(),
            }),
            EventType::BranchCreated => self.branches.push(Branch {
                name: e.aggregate.id.clone(),
                task_id: string_field(e, "task_id"),
            }),
            EventType::CommitObserved => self.commits.push(Commit {
                sha: e.aggregate.id.clone(),
                branch: string_field(e, "branch").unwrap_or_default(),
                message: string_field(e, "message").unwrap_or_default(),
                author: string_field(e, "author").unwrap_or_default(),
                task_id: string_field(e, "task_id"),
            }),
            EventType::MergeCompleted => self.merges.push(Merge {
                sha: e.aggregate.id.clone(),
                from_branch: string_field(e, "from_branch").unwrap_or_default(),
                to_branch: string_field(e, "to_branch").unwrap_or_default(),
            }),
            EventType::MergeConflictDetected => {
                // A merge conflict is an observation-like event — it's recorded
                // in the event log but doesn't add a persistent entity to the
                // projection (the PM reacts to it, it doesn't become a board
                // item). It WILL wake the PM (Tier-1 trigger).
            }
            EventType::ChangeSetReady => {
                // ChangeSetReady is handled in increment 3 (ChangeSet concept).
                // For now it's a signal event that the PM can react to.
            }
        }
    }
}

/// Helper to read a string field from an event's JSON `data`.
fn string_field(e: &Event, key: &str) -> Option<String> {
    e.data
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Helper to read a bool field from an event's JSON `data`.
fn bool_field(e: &Event, key: &str) -> Option<bool> {
    e.data.get(key).and_then(|v| v.as_bool())
}

/// Human label for an actor (owner / agent id / system).
fn actor_name(e: &Event) -> String {
    match &e.actor {
        crate::event::Actor::Owner => "owner".into(),
        crate::event::Actor::Agent { id } => id.clone(),
        crate::event::Actor::System => "system".into(),
    }
}
