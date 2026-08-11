//! Current-state projections, derived from the append-only event log.
//!
//! Per docs/CASTING_PROJECT_BRIEF.md §9/§14 and the handoff §4: event history is
//! the source of truth; these projections are recomputable, queryable current
//! state and are NEVER authoritative or stored-and-drifting. We rebuild them on
//! demand by folding the log (idempotent and fine for slice one).

use crate::event::{Event, EventType};
use crate::store::EventStore;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A hired consultant/agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Agent {
    pub id: String,
    pub role: String,
}

/// A product requirement the owner / PM agreed on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Requirement {
    pub id: String,
    pub title: String,
    pub description: String,
}

/// Where a task sits on the board (a projection of its lifecycle events).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    #[serde(rename = "backlog")]
    Backlog,
    #[serde(rename = "working")]
    Working,
    #[serde(rename = "in_review")]
    InReview,
    #[serde(rename = "blocked")]
    Blocked,
    #[serde(rename = "done")]
    Done,
}

/// A review verdict on a task (recorded when work passes through InReview).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskReview {
    pub reviewer: String,
    pub note: String,
    pub approved: bool,
}

/// A task. Status is derived from TaskCreated->TaskAssigned->TaskStarted->...->TaskCompleted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: TaskStatus,
    pub assignee: Option<String>,
    /// Current priority, reduced from `TaskPriorityChanged` events
    /// (defaults to Medium). Per docs/SEMANTIC_EVENTS.md this is derived state.
    pub priority: crate::plan::Priority,
    /// The review verdict (some once the task has passed through InReview).
    pub review: Option<TaskReview>,
}

/// A recordable decision and its eventual owner verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionStatus {
    #[serde(rename = "proposed")]
    Proposed,
    #[serde(rename = "approved")]
    Approved,
    #[serde(rename = "rejected")]
    Rejected,
    #[serde(rename = "superseded")]
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Decision {
    pub id: String,
    pub subject: String,
    pub options: serde_json::Value,
    pub recommendation: Option<String>,
    pub status: DecisionStatus,
    pub class: crate::policy::DecisionClass,
    pub involvement: crate::policy::OwnerInvolvement,
    /// Who decided this (Owner or an agent) once `DecisionMade` is recorded.
    pub decided_by: Option<String>,
    /// The decision that superseded this one, if any (history preserved).
    pub superseded_by: Option<String>,
    pub owner_verdict: Option<String>,
}

/// Human-readable message (owner <-> PM, agents). Structured, not real email.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: String,
    pub body: String,
}

/// Something an agent noticed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Observation {
    pub id: String,
    pub from: String,
    pub severity: String,
    pub subject: String,
    pub body: String,
}

/// Lifecycle of a first-class Risk object (SEMANTIC_EVENTS §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskStatus {
    Open,
    /// The risk came to pass; now a problem being handled.
    Materialized,
    Resolved,
}

/// A first-class project risk (semantic object). Creation may need the PM/LLM
/// to *interpret* an observation, but its state transitions are deterministic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Risk {
    pub id: String,
    pub subject: String,
    pub severity: String,
    pub status: RiskStatus,
    pub discovered_by: String,
}

/// A recorded project assumption (semantic note, SEMANTIC_EVENTS §8).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Assumption {
    pub id: String,
    pub body: String,
    pub recorded_by: String,
}

/// Lifecycle state of a recorded opinion (mirrors directives). An opinion is
/// `Active` until a later opinion explicitly supersedes it (never edited).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpinionStatus {
    /// Currently-held judgment / rationale / preference.
    Active,
    /// Replaced by a later opinion (history preserved, not overwritten).
    Superseded,
}

impl Default for OpinionStatus {
    /// Old/unknown opinions default to Active (records with no status field are
    /// treated as currently-held).
    fn default() -> Self {
        OpinionStatus::Active
    }
}

/// A recorded project OPINION — a subjective judgment / rationale / preference
/// (e.g. "Postgres is a good default for our event log"). Subjective: a later
/// opinion supersedes it (history preserved) rather than editing it in place.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Opinion {
    pub id: String,
    /// The thing this opinion is ABOUT — the matching key for drift/supersession
    /// (e.g. "databases", "auth"). Distinct from `category` (rationale/design/
    /// lesson/preference, a free tag). Empty means ungroupable; the reconciler
    /// skips opinions with no subject.
    pub subject: String,
    /// Category: "rationale" | "design" | "lesson" | "preference" (free string,
    /// not an enum, so categories can evolve without a migration).
    pub category: String,
    pub statement: String,
    pub recorded_by: String,
    /// Current validity. Derived in the projection: `Active` until an
    /// `OpinionSuperseded` event flips this to `Superseded`. Readers that want
    /// only currently-valid opinions filter on `Active`.
    #[serde(default)]
    pub status: OpinionStatus,
    /// If non-empty, the id of the opinion this one supersedes.
    #[serde(default)]
    pub supersedes: Option<String>,
}

/// A recorded project FACT — an objective, measured point-in-time datapoint (e.g. "the repo
/// is 1,342 lines"). Objective measures are usually derived from state; this
/// captures a point-in-time snapshot worth preserving.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fact {
    pub id: String,
    /// What kind of measurement, e.g. "loc" | "events" | "tasks" (free string).
    pub kind: String,
    pub statement: String,
    pub recorded_by: String,
    /// ISO/chrono timestamp at record time, so a datapoint is a point-in-time.
    pub recorded_at: String,
}

/// One cost-entry: provider metering for an agent/model call (HARNESS #6).
/// Kept in the projection so spend is attributable per agent/task and the PM's
/// budget concern reads it — the event log remains the durable authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CostEntry {
    pub id: String,
    /// The agent whose call incurred this spend.
    pub agent_id: String,
    /// The task this spend is attributed to, if any.
    #[serde(default)]
    pub task_id: Option<String>,
    /// Model tier, e.g. "flash" | "pro" (free string from the provider).
    pub model_tier: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Estimated USD cost of this call.
    pub estimated_usd: f64,
    /// Timestamp so spend is queryable over time.
    pub incurred_at: String,
}

/// A recorded project constraint (semantic note, SEMANTIC_EVENTS §8).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Constraint {
    pub id: String,
    pub body: String,
    pub recorded_by: String,
}

/// A branch in the artifact repo (semantic Git event, ADDENDUM §20/§23).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Branch {
    pub name: String,
    /// The task this branch is associated with, if known (ADDENDUM §20).
    pub task_id: Option<String>,
}

/// A commit observed on a branch (semantic Git event, ADDENDUM §23).
/// Git remains authoritative for the commit; Casting owns the organizational
/// association (ADDENDUM §24).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Merge {
    /// The merge commit sha.
    pub sha: String,
    /// The branch that was merged.
    pub from_branch: String,
    /// The branch that received the merge.
    pub to_branch: String,
}

/// The status of a ChangeSet (ADDENDUM §22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeSetStatus {
    #[serde(rename = "open")]
    /// Branch exists, commits being produced, not yet ready for review.
    Open,
    #[serde(rename = "ready")]
    /// Ready for review (ChangeSetReady emitted).
    Ready,
    #[serde(rename = "merged")]
    /// Merged into a protected branch.
    Merged,
}

/// A ChangeSet — the unit of agent output: which task, branch, and commits
/// produced a batch of work (ADDENDUM §21–22). Git remains authoritative for
/// the branch and commits; Casting owns the association.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChangeSet {
    pub id: String,
    /// The task this ChangeSet fulfills.
    pub task_id: String,
    /// The branch the work lives on (e.g. `casting/task-381-authentication`).
    pub branch: String,
    /// The commit shas on this branch (in chronological order).
    pub commits: Vec<String>,
    /// The agent who produced this work.
    pub agent: Option<String>,
    pub status: ChangeSetStatus,
}

/// The full current-state projection for a project.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Projection {
    pub project_id: String,
    pub agents: Vec<Agent>,
    pub requirements: Vec<Requirement>,
    pub tasks: Vec<Task>,
    pub decisions: Vec<Decision>,
    pub messages: Vec<Message>,
    pub observations: Vec<Observation>,
    /// First-class semantic objects (SEMANTIC_EVENTS §8).
    pub risks: Vec<Risk>,
    pub assumptions: Vec<Assumption>,
    pub constraints: Vec<Constraint>,
    /// Recorded project opinions (subjective knowledge worth not re-deriving).
    pub opinions: Vec<Opinion>,
    /// Recorded project facts (objective, point-in-time measures).
    pub facts: Vec<Fact>,
    /// Cost entries (HARNESS #6): provider metering so spend is attributable.
    pub spend: Vec<CostEntry>,
    /// First-class governance objects (docs/INTENT.md).
    pub directives: Vec<crate::directive::Directive>,
    /// Branches in the artifact repo (semantic Git events).
    pub branches: Vec<Branch>,
    /// Commits observed on branches (semantic Git events).
    pub commits: Vec<Commit>,
    /// Completed merges (semantic Git events).
    pub merges: Vec<Merge>,
    /// ChangeSets — the unit of agent output (ADDENDUM §21–22).
    pub changesets: Vec<ChangeSet>,
    /// The project's decision policy (delegated authority, brief §5),
    /// folded from `DecisionPolicyChanged` events. Event-sourced: the owner's
    /// per-class autonomy configuration is durable history, not a default.
    pub policy: crate::policy::DecisionPolicy,
    /// The derived Project Plan (objective + ranked priorities + open
    /// decisions). Recomputed at build() from the folded projection — this is
    /// current state, never stored authoritative (SEMANTIC_EVENTS.md).
    pub plan: crate::plan::ProjectPlan,
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
        // Derive the current plan from the folded state (recomputed, never
        // stored authoritative).
        p.plan = p.plan();
        Ok(p)
    }

    /// Currently-valid opinions (status == Active). The derived "what is
    /// actually believed right now" view — superseded opinions are excluded
    /// but preserved in `self.opinions` for history. Readers that want the
    /// full audit trail (incl. superseded) can read `self.opinions` directly.
    pub fn active_opinions(&self) -> Vec<&Opinion> {
        self.opinions
            .iter()
            .filter(|o| o.status == OpinionStatus::Active)
            .collect()
    }

    /// Currently-valid opinions in a given category (status == Active), so a
    /// reader can ask e.g. "what's the current design rationale?".
    pub fn active_opinions_by_category(&self, category: &str) -> Vec<&Opinion> {
        self.active_opinions()
            .into_iter()
            .filter(|o| o.category == category)
            .collect()
    }

    /// Total token spend across all cost entries (HARNESS #6 budget concern).
    pub fn total_prompt_tokens(&self) -> u64 {
        self.spend.iter().map(|c| c.prompt_tokens).sum()
    }

    /// Total cost in USD across all cost entries.
    pub fn total_spend_usd(&self) -> f64 {
        self.spend.iter().map(|c| c.estimated_usd).sum()
    }

    /// Total spend attributed to one agent (for per-consultant budgeting).
    pub fn spend_by_agent(&self, agent_id: &str) -> f64 {
        self.spend
            .iter()
            .filter(|c| c.agent_id == agent_id)
            .map(|c| c.estimated_usd)
            .sum()
    }

    /// Apply a single event to the running projection.
    pub fn apply(&mut self, e: &Event) {
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
                priority: crate::plan::Priority::default(),
                review: None,
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
            EventType::TaskReadyForReview => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.id == e.aggregate.id) {
                    task.status = TaskStatus::InReview;
                }
            }
            EventType::TaskReviewed => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.id == e.aggregate.id) {
                    let approved = bool_field(e, "approved").unwrap_or(false);
                    task.review = Some(TaskReview {
                        reviewer: actor_name(e),
                        note: string_field(e, "note").unwrap_or_default(),
                        approved,
                    });
                    task.status = if approved {
                        TaskStatus::Done
                    } else {
                        // Rejected -> rework: back to Working.
                        TaskStatus::Working
                    };
                }
            }
            EventType::TaskPriorityChanged => {
                // Deterministic reducer: only `to` matters for current state.
                // (`from` is kept in the event for history richness.)
                if let (Some(task), Some(to)) = (
                    self.tasks.iter_mut().find(|t| t.id == e.aggregate.id),
                    e.data
                        .get("to")
                        .and_then(|v| serde_json::from_value(v.clone()).ok()),
                ) {
                    task.priority = to;
                }
            }
            EventType::ObservationCreated => self.observations.push(Observation {
                id: e.aggregate.id.clone(),
                from: actor_name(e),
                severity: string_field(e, "severity").unwrap_or_else(|| "info".into()),
                subject: string_field(e, "subject").unwrap_or_default(),
                body: string_field(e, "body").unwrap_or_default(),
            }),
            EventType::RiskRaised => self.risks.push(Risk {
                id: e.aggregate.id.clone(),
                subject: string_field(e, "subject").unwrap_or_default(),
                severity: string_field(e, "severity").unwrap_or_else(|| "medium".into()),
                status: RiskStatus::Open,
                discovered_by: actor_name(e),
            }),
            EventType::RiskUpdated => {
                if let Some(risk) = self.risks.iter_mut().find(|r| r.id == e.aggregate.id) {
                    if let Some(status) = e
                        .data
                        .get("status")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                    {
                        risk.status = status;
                    }
                }
            }
            EventType::AssumptionRecorded => self.assumptions.push(Assumption {
                id: e.aggregate.id.clone(),
                body: string_field(e, "body").unwrap_or_default(),
                recorded_by: actor_name(e),
            }),
            EventType::ConstraintRecorded => self.constraints.push(Constraint {
                id: e.aggregate.id.clone(),
                body: string_field(e, "body").unwrap_or_default(),
                recorded_by: actor_name(e),
            }),
            EventType::OpinionRecorded => self.opinions.push(Opinion {
                id: e.aggregate.id.clone(),
                subject: string_field(e, "subject").unwrap_or_default(),
                category: string_field(e, "category").unwrap_or_default(),
                statement: string_field(e, "statement").unwrap_or_default(),
                recorded_by: actor_name(e),
                status: OpinionStatus::Active,
                supersedes: string_field(e, "supersedes"),
            }),
            EventType::OpinionSuperseded => {
                if let Some(op) = self.opinions.iter_mut().find(|o| o.id == e.aggregate.id) {
                    op.status = OpinionStatus::Superseded;
                }
            }
            EventType::FactRecorded => self.facts.push(Fact {
                id: e.aggregate.id.clone(),
                kind: string_field(e, "kind").unwrap_or_default(),
                statement: string_field(e, "statement").unwrap_or_default(),
                recorded_by: actor_name(e),
                recorded_at: e.timestamp.to_string(),
            }),
            EventType::CostIncurred => {
                let num_field =
                    |k: &str| -> u64 { e.data.get(k).and_then(|v| v.as_u64()).unwrap_or(0) };
                self.spend.push(CostEntry {
                    id: e.aggregate.id.clone(),
                    agent_id: string_field(e, "agent_id").unwrap_or_default(),
                    task_id: string_field(e, "task_id"),
                    model_tier: string_field(e, "model_tier").unwrap_or_default(),
                    prompt_tokens: num_field("prompt_tokens"),
                    completion_tokens: num_field("completion_tokens"),
                    estimated_usd: e
                        .data
                        .get("estimated_usd")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    incurred_at: e.timestamp.to_string(),
                });
            }
            EventType::ProjectDirectiveCreated => {
                use crate::directive::{Directive, DirectiveKind, DirectiveStrength};
                let kind: Option<DirectiveKind> = e
                    .data
                    .get("kind")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let strength: Option<DirectiveStrength> = e
                    .data
                    .get("strength")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let scope: Vec<String> = e
                    .data
                    .get("scope")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                self.directives.push(Directive::new(
                    e.aggregate.id.clone(),
                    kind.unwrap_or(DirectiveKind::Policy),
                    string_field(e, "statement").unwrap_or_default(),
                    scope,
                    strength.unwrap_or(DirectiveStrength::Recommended),
                    string_field(e, "created_by").unwrap_or_else(|| actor_name(e)),
                    string_field(e, "supersedes"),
                ));
            }
            EventType::ProjectDirectiveSuspended => {
                if let Some(d) = self.directives.iter_mut().find(|d| d.id == e.aggregate.id) {
                    d.status = crate::directive::DirectiveStatus::Suspended;
                }
            }
            EventType::ProjectDirectiveResumed => {
                if let Some(d) = self.directives.iter_mut().find(|d| d.id == e.aggregate.id) {
                    d.status = crate::directive::DirectiveStatus::Active;
                }
            }
            EventType::ProjectDirectiveSuperseded => {
                if let Some(d) = self.directives.iter_mut().find(|d| d.id == e.aggregate.id) {
                    d.status = crate::directive::DirectiveStatus::Superseded;
                }
            }
            EventType::ProjectDirectiveExpired => {
                if let Some(d) = self.directives.iter_mut().find(|d| d.id == e.aggregate.id) {
                    d.status = crate::directive::DirectiveStatus::Expired;
                }
            }
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
                class: e
                    .data
                    .get("class")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(crate::policy::DecisionClass::InternalImplementation),
                involvement: e
                    .data
                    .get("involvement")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(crate::policy::OwnerInvolvement::Ask),
                decided_by: None,
                superseded_by: None,
                owner_verdict: None,
            }),
            EventType::DecisionMade => {
                // The aggregate id is the decision being ruled on. `DecisionMade`
                // is the universal decision-maker event: the actor is who decided
                // (Owner after being asked, or a delegated PM/agent).
                if let Some(dec) = self.decisions.iter_mut().find(|d| d.id == e.aggregate.id) {
                    let approved = bool_field(e, "approved").unwrap_or(false);
                    dec.status = if approved {
                        DecisionStatus::Approved
                    } else {
                        DecisionStatus::Rejected
                    };
                    dec.decided_by = Some(actor_name(e));
                    dec.owner_verdict = string_field(e, "note");
                }
            }
            EventType::DecisionSuperseded => {
                if let Some(dec) = self.decisions.iter_mut().find(|d| d.id == e.aggregate.id) {
                    dec.status = DecisionStatus::Superseded;
                    dec.superseded_by = string_field(e, "superseded_by");
                }
            }
            EventType::MessageSent => self.messages.push(Message {
                id: e.aggregate.id.clone(),
                from: actor_name(e),
                to: string_field(e, "to").unwrap_or_else(|| "owner".into()),
                body: string_field(e, "body").unwrap_or_default(),
            }),
            EventType::DecisionPolicyChanged => {
                // Rebind the owner-involvement for the decision class (brief §5).
                // Event-sourced: the projection's policy is derived from the log.
                if let (Some(class), Some(involvement)) = (
                    e.data
                        .get("class")
                        .and_then(|v| serde_json::from_value(v.clone()).ok()),
                    e.data
                        .get("involvement")
                        .and_then(|v| serde_json::from_value(v.clone()).ok()),
                ) {
                    self.policy.set(class, involvement);
                }
            }
            EventType::BranchCreated => {
                let name = e.aggregate.id.clone();
                let task_id = string_field(e, "task_id");
                self.branches.push(Branch {
                    name: name.clone(),
                    task_id: task_id.clone(),
                });
                // Auto-derive an Open ChangeSet when a task branch appears
                // (ADDENDUM §20–22). The ChangeSet id is derived from the
                // task id so it's stable and discoverable.
                if let Some(tid) = &task_id {
                    let cs_id = format!("changeset-{tid}");
                    if !self.changesets.iter().any(|c| c.id == cs_id) {
                        self.changesets.push(ChangeSet {
                            id: cs_id,
                            task_id: tid.clone(),
                            branch: name,
                            commits: Vec::new(),
                            agent: None,
                            status: ChangeSetStatus::Open,
                        });
                    }
                }
            }
            EventType::CommitObserved => {
                let sha = e.aggregate.id.clone();
                let branch = string_field(e, "branch").unwrap_or_default();
                let task_id = string_field(e, "task_id");
                self.commits.push(Commit {
                    sha: sha.clone(),
                    branch: branch.clone(),
                    message: string_field(e, "message").unwrap_or_default(),
                    author: string_field(e, "author").unwrap_or_default(),
                    task_id: task_id.clone(),
                });
                // Append the commit to its ChangeSet if one exists for this
                // task (auto-derived from the branch). This keeps the
                // ChangeSet's commit list in sync as commits arrive.
                if let Some(tid) = &task_id {
                    let cs_id = format!("changeset-{tid}");
                    if let Some(cs) = self.changesets.iter_mut().find(|c| c.id == cs_id) {
                        if !cs.commits.contains(&sha) {
                            cs.commits.push(sha);
                        }
                    }
                }
            }
            EventType::ChangeSetReady => {
                // A ChangeSet is explicitly assembled and marked ready.
                let id = e.aggregate.id.clone();
                let task_id = string_field(e, "task_id").unwrap_or_default();
                let branch = string_field(e, "branch").unwrap_or_default();
                let agent = string_field(e, "agent");
                let commits: Vec<String> = e
                    .data
                    .get("commits")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                if let Some(cs) = self.changesets.iter_mut().find(|c| c.id == id) {
                    // Update existing ChangeSet to Ready.
                    cs.status = ChangeSetStatus::Ready;
                    if !commits.is_empty() {
                        cs.commits = commits;
                    }
                    if agent.is_some() {
                        cs.agent = agent;
                    }
                } else {
                    self.changesets.push(ChangeSet {
                        id,
                        task_id,
                        branch,
                        commits,
                        agent,
                        status: ChangeSetStatus::Ready,
                    });
                }
            }
            EventType::MergeCompleted => {
                // A merge into a protected branch marks a ChangeSet as Merged
                // if the merged branch matches one.
                let merged_branch = string_field(e, "from_branch").unwrap_or_default();
                for cs in self.changesets.iter_mut() {
                    if cs.branch == merged_branch {
                        cs.status = ChangeSetStatus::Merged;
                    }
                }
                // Also record the merge as before.
                self.merges.push(Merge {
                    sha: e.aggregate.id.clone(),
                    from_branch: string_field(e, "from_branch").unwrap_or_default(),
                    to_branch: string_field(e, "to_branch").unwrap_or_default(),
                });
            }
            EventType::MergeConflictDetected => {
                // A merge conflict is an observation-like event — it's recorded
                // in the event log but doesn't add a persistent entity to the
                // projection (the PM reacts to it, it doesn't become a board
                // item). It WILL wake the PM (Tier-1 trigger).
            }
        }
    }
}

/// Derived Project Plan — the deterministic "current state" of what we're
/// building (docs/SEMANTIC_EVENTS.md §9): objective, tasks ranked by priority,
/// deprioritized, and decisions awaiting the owner. Recomputed from the
/// projection; never stored authoritative.
impl Projection {
    pub fn plan(&self) -> crate::plan::ProjectPlan {
        use crate::plan::{PlannedItem, Priority};

        let objective = self.requirements.last().map(|r| r.title.clone());

        // Current work: open (not done/blocked) tasks with an assigned priority,
        // ranked critical..low. Ties broken by insertion order (stable).
        let mut tasks: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Backlog || t.status == TaskStatus::Working)
            .collect();
        tasks.sort_by_key(|t| std::cmp::Reverse(t.priority));

        let open_decisions: Vec<String> = self
            .decisions
            .iter()
            .filter(|d| d.status == DecisionStatus::Proposed)
            .map(|d| d.subject.clone())
            .collect();

        // Open risks (not yet resolved) — semantic objects surfaced in the plan.
        let open_risks: Vec<String> = self
            .risks
            .iter()
            .filter(|r| r.status == crate::projection::RiskStatus::Open)
            .map(|r| r.subject.clone())
            .collect();

        // Active governing directives, strongest-first (governance surfaced).
        use crate::directive::DirectiveStatus;
        let mut active = self
            .directives
            .iter()
            .filter(|d| d.status == DirectiveStatus::Active)
            .collect::<Vec<_>>();
        active.sort_by_key(|d| std::cmp::Reverse(d.strength));
        let active_directives: Vec<String> = active
            .iter()
            .map(|d| format!("[{}] {}", d.kind.label(), d.statement))
            .collect();

        // Deprioritized = the lowest-priority open tasks (Low).
        let deprioritized: Vec<PlannedItem> = tasks
            .iter()
            .filter(|t| t.priority == Priority::Low)
            .map(|t| PlannedItem {
                task_id: t.id.clone(),
                title: t.title.clone(),
                priority: t.priority,
            })
            .collect();

        crate::plan::ProjectPlan {
            objective,
            priorities: tasks
                .iter()
                .map(|t| PlannedItem {
                    task_id: t.id.clone(),
                    title: t.title.clone(),
                    priority: t.priority,
                })
                .collect(),
            deprioritized,
            open_risks,
            active_directives,
            open_decisions,
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
