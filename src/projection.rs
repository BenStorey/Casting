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

pub use crate::types::{
    Agent, Assumption, Branch, Briefing, BriefingAsset, BriefingStatus, ChangeSet, ChangeSetStatus,
    Commit, Constraint, CostEntry, Decision, DecisionStatus, Diagram, ExternalRequest,
    ExternalRequestStatus, Fact, Merge, Message, Observation, Opinion, OpinionStatus, Requirement,
    Risk, RiskStatus, Task, TaskReview, TaskStatus, Worktree,
};

/// The full current-state projection for a project.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Projection {
    pub project_id: String,
    pub agents: Vec<Agent>,
    pub requirements: Vec<Requirement>,
    pub tasks: Vec<Task>,
    pub decisions: Vec<Decision>,
    pub messages: Vec<Message>,
    /// The owner↔advisor private thread. ISOLATED from PM context by design —
    /// only reaches the PM via an `AdvisorHandoff` (which becomes a Briefing).
    pub advisor_thread: Vec<Message>,
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
    /// External advisor briefings imported into the project (advisory, NOT
    /// authoritative — see `Briefing`).
    pub briefings: Vec<Briefing>,
    /// External requests (product intake surface): issues/PRs raised outside,
    /// carrying provenance so the PM can triage them. See `ExternalRequest`.
    pub external_requests: Vec<ExternalRequest>,
    /// Diagrams drawn + saved in the app (Excalidraw). See `Diagram`.
    pub diagrams: Vec<Diagram>,
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
    /// Isolated worktrees provisioned for summoned consultants (owner,
    /// 2026-08-12). One per task; the platform provisions them so concurrent
    /// consultants can't collide (distinct branch/build-target/port).
    pub worktrees: Vec<Worktree>,
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

    /// Deterministic triage of an external request (intake surface, D2-free):
    /// classify bug/feature/security, estimate severity, and detect duplicates.
    pub fn triage_request(
        &self,
        source: &str,
        external_id: Option<&str>,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> (String, String, bool) {
        // Classification + severity come from the single source of truth
        // (crate::triage) — the same one that stamps the ExternalRequestReceived
        // event, so this read-side verdict can never disagree with the log.
        let (classification, severity) = crate::triage::classify(title, body, labels);

        // Duplicate: same source + external_id, or same source + normalized (lowercased) title.
        let dup = self.external_requests.iter().any(|r| {
            (r.source == source
                && r.external_id.is_some()
                && r.external_id == external_id.map(str::to_owned))
                || (r.source == source
                    && !r.title.is_empty()
                    && r.title.to_lowercase() == title.trim().to_lowercase())
        });

        (classification.to_string(), severity.to_string(), dup)
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
            EventType::AdvisoryBriefingImported => {
                fn str(e: &Event, k: &str) -> String {
                    string_field(e, k).unwrap_or_default()
                }
                let assets: Vec<BriefingAsset> = e
                    .data
                    .get("assets")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                self.briefings.push(Briefing {
                    id: e.aggregate.id.clone(),
                    source: str(e, "source"),
                    subject: str(e, "subject"),
                    title: str(e, "title"),
                    body: str(e, "body"),
                    assets,
                    brought_in_by: str(e, "brought_in_by"),
                    status: BriefingStatus::Active,
                    supersedes: string_field(e, "supersedes"),
                    imported_at: e.timestamp.to_string(),
                });
            }
            EventType::ExternalRequestReceived => {
                fn str(e: &Event, k: &str) -> String {
                    string_field(e, k).unwrap_or_default()
                }
                let labels: Vec<String> = e
                    .data
                    .get("labels")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let classification = str(e, "classification");
                let severity = str(e, "severity");
                self.external_requests.push(ExternalRequest {
                    id: e.aggregate.id.clone(),
                    source: str(e, "source"),
                    external_id: string_field(e, "external_id"),
                    title: str(e, "title"),
                    body: str(e, "body"),
                    reporter: str(e, "reporter"),
                    labels,
                    url: string_field(e, "url"),
                    classification,
                    severity,
                    status: ExternalRequestStatus::Open,
                    received_at: e.timestamp.to_string(),
                });
            }
            EventType::DiagramSaved => {
                self.diagrams.push(Diagram {
                    id: e.aggregate.id.clone(),
                    title: string_field(e, "title").unwrap_or_default(),
                    data: string_field(e, "data").unwrap_or_default(),
                    saved_by: string_field(e, "saved_by").unwrap_or_default(),
                    saved_at: e.timestamp.to_string(),
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
            EventType::AdvisorMessageSent => {
                self.advisor_thread.push(Message {
                    id: e.aggregate.id.clone(),
                    from: actor_name(e),
                    to: string_field(e, "to").unwrap_or_else(|| "advisor".into()),
                    body: string_field(e, "body").unwrap_or_default(),
                });
            }
            EventType::AdvisorHandoff => {
                // The owner turned the private advisor thread into a Briefing the
                // PM reads: advisory (source "advisor"), summarizing the thread.
                self.briefings.push(Briefing {
                    id: e.aggregate.id.clone(),
                    source: "advisor".to_string(),
                    subject: string_field(e, "subject").unwrap_or_default(),
                    title: string_field(e, "title").unwrap_or_default(),
                    body: string_field(e, "body").unwrap_or_default(),
                    assets: Vec::new(),
                    brought_in_by: "owner".to_string(),
                    status: BriefingStatus::Active,
                    supersedes: None,
                    imported_at: e.timestamp.to_string(),
                });
            }
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
            EventType::WorktreeProvisioned => {
                // The platform provisioned an isolated workspace for a task.
                // Record it, and auto-create/refresh the Open ChangeSet with
                // the EXACT branch mapping (no derive_task_id guessing — the
                // platform knows the association because it created it).
                let task_id = string_field(e, "task_id").unwrap_or_default();
                let branch = string_field(e, "branch").unwrap_or_default();
                let path = string_field(e, "path").unwrap_or_default();
                let cargo_target_dir = string_field(e, "cargo_target_dir").unwrap_or_default();
                let port = e.data.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
                if !task_id.is_empty() {
                    let wt = Worktree {
                        task_id: task_id.clone(),
                        branch: branch.clone(),
                        path,
                        cargo_target_dir,
                        port,
                    };
                    if let Some(existing) = self.worktrees.iter_mut().find(|w| w.task_id == task_id)
                    {
                        *existing = wt; // refresh (idempotent re-provision)
                    } else {
                        self.worktrees.push(wt);
                    }
                    // Auto-create an Open ChangeSet for the task if none yet.
                    let cs_id = format!("changeset-{task_id}");
                    if !self.changesets.iter().any(|c| c.id == cs_id) {
                        self.changesets.push(ChangeSet {
                            id: cs_id,
                            task_id: task_id.clone(),
                            branch,
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
