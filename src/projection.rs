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
    ActionRejection, Agent, Assumption, Branch, Briefing, BriefingAsset, BriefingStatus, ChangeSet,
    ChangeSetStatus, Commit, Constraint, CostEntry, CoverageInfo, Decision, DecisionStatus,
    Diagram, ExternalRequest, ExternalRequestStatus, Fact, LanguageLines, Merge, Message,
    Observation, Opinion, OpinionStatus, OrchestrationRun, RepoMetrics, Requirement, Risk,
    RiskStatus, Task, TaskDependency, TaskReview, TaskStatus, Worktree,
};

/// The full current-state projection for a project.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Projection {
    pub project_id: String,
    pub agents: Vec<Agent>,
    pub requirements: Vec<Requirement>,
    pub tasks: Vec<Task>,
    /// Hard dependency edges between tasks (task -> waits on blocking_task until
    /// a state). Folded from `TaskBlockedOn` events. NEVER a side table — this
    /// is derived state, the event log is the authority.
    pub dependencies: Vec<TaskDependency>,
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
    /// The owner-set hard token budget (guard circuit breaker). None = unset.
    pub budget: Option<crate::guard::Budget>,
    /// A resumable pause in effect (owner- or watchdog-set). None = running.
    pub paused: Option<crate::guard::PauseInfo>,
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
    /// Point-in-time repo-metric snapshots, one per PR landing (per-merge
    /// trend: file count, lines by language, best-effort coverage).
    pub repo_metrics: Vec<RepoMetrics>,
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
    /// Archived terminal entities — compact summaries replacing old closed
    /// state in the active projection. Filled from `EntityArchived` events.
    /// Agents omit entities whose id is in this set from their context.
    #[serde(default)]
    pub archived: Vec<crate::types::ArchivedRecord>,
    /// The derived Project Plan (objective + ranked priorities + open
    /// decisions). Recomputed at build() from the folded projection — this is
    /// current state, never stored authoritative (SEMANTIC_EVENTS.md).
    pub plan: crate::plan::ProjectPlan,
    /// Diagnostics audit trail (2026-08): refused PM actions (`PlanActionRejected`)
    /// and recorded orchestrator planning passes (`OrchestrationRun`). Derived
    /// read-side records so misbehaving plans are visible in the UI, never
    /// just printed to a server log.
    pub rejections: Vec<ActionRejection>,
    pub orchestration: Vec<OrchestrationRun>,
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
                merge_authority: crate::types::MergeAuthority::default(),
                priority: crate::plan::Priority::default(),
                review: None,
                parent_id: string_field(e, "parent_id"),
            }),
            EventType::TaskAssigned => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.id == e.aggregate.id) {
                    task.assignee = string_field(e, "assignee");
                    task.merge_authority = e
                        .data
                        .get("merge_authority")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                }
            }
            // Reclassification of the merge decision (tiered merge policy).
            EventType::MergeAuthorityChanged => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.id == e.aggregate.id) {
                    if let Some(to) = e
                        .data
                        .get("to")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                    {
                        task.merge_authority = to;
                    }
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
            // TaskDecomposed carries no projector state of its own: the child
            // TaskCreated events (each carrying `parent_id`) already fold into
            // `self.tasks`. The decomposition event is pure provenance — the
            // graph reconstructs the structure from tasks' parent_id links.
            EventType::TaskDecomposed => {}
            // A hard dependency edge: `task` (aggregate id) waits on
            // `blocking_task` until it reaches `required_state`.
            EventType::TaskBlockedOn => {
                let required_state = e
                    .data
                    .get("required_state")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(TaskStatus::Done);
                self.dependencies.push(TaskDependency {
                    task: e.aggregate.id.clone(),
                    blocking_task: string_field(e, "blocking_task_id").unwrap_or_default(),
                    required_state,
                });
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
                    model: string_field(e, "model"),
                    provider: string_field(e, "provider"),
                    prompt_tokens: num_field("prompt_tokens"),
                    completion_tokens: num_field("completion_tokens"),
                    cache_read_input_tokens: num_field("cache_read_input_tokens"),
                    cache_creation_input_tokens: num_field("cache_creation_input_tokens"),
                    latency_ms: num_field("latency_ms"),
                    input_price_per_mtok: e
                        .data
                        .get("input_price_per_mtok")
                        .and_then(|v| v.as_f64()),
                    output_price_per_mtok: e
                        .data
                        .get("output_price_per_mtok")
                        .and_then(|v| v.as_f64()),
                    estimated_usd: e
                        .data
                        .get("estimated_usd")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    incurred_at: e.timestamp.to_string(),
                });
            }
            EventType::BudgetSet => {
                let limit_usd = e
                    .data
                    .get("limit_usd")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let warn_at = e
                    .data
                    .get("warn_at")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.80);
                self.budget = Some(crate::guard::Budget { limit_usd, warn_at });
            }
            EventType::WorkPaused => {
                self.paused = Some(crate::guard::PauseInfo {
                    reason: string_field(e, "reason").unwrap_or_default(),
                    by: string_field(e, "by").unwrap_or_default(),
                    at: e.timestamp.to_string(),
                });
            }
            EventType::WorkResumed => {
                self.paused = None;
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
            // Durable execution events fold to NOTHING in the projection: they are
            // durable records the executor queries by scanning the event log
            // (`executor::has_completed` / `redispatch_inflight`). No projection
            // state is needed — the log is the authority, and the executor IS the
            // only consumer of these events.
            EventType::ActivityScheduled => {}
            EventType::ActivityCompleted => {}
            EventType::ActivityFailed => {}
            EventType::PlanActionRejected => {
                self.rejections.push(ActionRejection {
                    who: string_field(e, "who").unwrap_or_default(),
                    action: string_field(e, "action").unwrap_or_default(),
                    reason: string_field(e, "reason").unwrap_or_default(),
                    correlation: string_field(e, "correlation"),
                    at: e.timestamp.to_string(),
                });
            }
            EventType::OrchestrationRun => {
                let planned: Vec<String> = e
                    .data
                    .get("planned")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                self.orchestration.push(OrchestrationRun {
                    trigger: string_field(e, "trigger").unwrap_or_default(),
                    actor: string_field(e, "actor").unwrap_or_default(),
                    correlation: string_field(e, "correlation").unwrap_or_default(),
                    context_summary: string_field(e, "context_summary").unwrap_or_default(),
                    planned,
                    metered: e
                        .data
                        .get("metered")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    metering_agent: string_field(e, "metering_agent"),
                    provider: string_field(e, "provider"),
                    model: string_field(e, "model"),
                    prompt_tokens: e
                        .data
                        .get("prompt_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    completion_tokens: e
                        .data
                        .get("completion_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    latency_ms: e
                        .data
                        .get("latency_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    estimated_usd: e
                        .data
                        .get("estimated_usd")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    at: e.timestamp.to_string(),
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
                // The platform provisioned an isolated workspace for a consultant.
                // In the NEW persistent model, each consultant gets N worktree
                // slots at setup (consultant + slot + port, no task_id yet).
                // The OLD per-task model carries task_id — we handle both.
                let consultant = string_field(e, "consultant").unwrap_or_default();
                let slot = e.data.get("slot").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let task_id = string_field(e, "task_id");
                let branch = string_field(e, "branch").unwrap_or_default();
                let path = string_field(e, "path").unwrap_or_default();
                let cargo_target_dir = string_field(e, "cargo_target_dir").unwrap_or_default();
                let port = e.data.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
                // Build the unique key for dedup: consultant+slot or task_id.
                let key = if !consultant.is_empty() {
                    format!("{consultant}+{slot}")
                } else if let Some(tid) = &task_id {
                    tid.clone()
                } else {
                    return; // no key — skip.
                };
                let wt = Worktree {
                    consultant: consultant.clone(),
                    slot,
                    task_id: task_id.clone(),
                    branch: branch.clone(),
                    path,
                    cargo_target_dir,
                    port,
                };
                // Upsert by key (idempotent re-provision).
                let existing_pos = if !consultant.is_empty() {
                    self.worktrees
                        .iter()
                        .position(|w| w.consultant == consultant && w.slot == slot)
                } else {
                    self.worktrees.iter().position(|w| w.task_id == task_id)
                };
                if let Some(pos) = existing_pos {
                    self.worktrees[pos] = wt;
                } else {
                    self.worktrees.push(wt);
                }
                // Auto-create an Open ChangeSet for the task if one is bound.
                // (Only in the old per-task model where task_id is known at provision.)
                if let Some(ref tid) = task_id {
                    let cs_id = format!("changeset-{tid}");
                    if !self.changesets.iter().any(|c| c.id == cs_id) {
                        self.changesets.push(ChangeSet {
                            id: cs_id,
                            task_id: tid.clone(),
                            branch,
                            commits: Vec::new(),
                            agent: None,
                            status: ChangeSetStatus::Open,
                        });
                    }
                }
            }
            EventType::WorktreeRemoved => {
                // Lifecycle close: dropping the Worktree from the projection
                // (OLD per-task model). In the NEW persistent model, worktrees
                // are not removed — they are released via WorktreeReleased.
                let task_id = string_field(e, "task_id").unwrap_or_default();
                if !task_id.is_empty() {
                    self.worktrees.retain(|w| w.task_id.as_deref() != Some(&task_id));
                }
            }
            EventType::WorktreeBound => {
                // A task is bound to a persistent worktree slot.
                let consultant = string_field(e, "consultant").unwrap_or_default();
                let slot = e.data.get("slot").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let task_id = string_field(e, "task_id").unwrap_or_default();
                let branch = string_field(e, "branch").unwrap_or_default();
                if !consultant.is_empty() && !task_id.is_empty() {
                    if let Some(wt) = self
                        .worktrees
                        .iter_mut()
                        .find(|w| w.consultant == consultant && w.slot == slot)
                    {
                        wt.task_id = Some(task_id.clone());
                        wt.branch = branch.clone();
                    }
                    // Auto-create an Open ChangeSet for the bound task.
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
            EventType::WorktreeReleased => {
                // A task is released from a persistent worktree slot (done/merged).
                let consultant = string_field(e, "consultant").unwrap_or_default();
                let slot = e.data.get("slot").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let task_id = string_field(e, "task_id").unwrap_or_default();
                if !consultant.is_empty() {
                    if let Some(wt) = self
                        .worktrees
                        .iter_mut()
                        .find(|w| w.consultant == consultant && w.slot == slot)
                    {
                        wt.task_id = None;
                        wt.branch = "main".into();
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
                    additions: e
                        .data
                        .get("additions")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    deletions: e
                        .data
                        .get("deletions")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    files: e.data.get("files").and_then(|v| v.as_u64()).unwrap_or(0),
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
            EventType::CommitRequested => {
                // Provenance: the assignee chose to checkpoint their WIP. The
                // actual commit lands as a CommitObserved once the git runner
                // makes it; this arm records the intent. No state mutation is
                // strictly needed (the ChangeSet commit list is filled by
                // CommitObserved), so it's a no-op fold for now — present so
                // the reducer match is exhaustive and the event is visible.
                let _ = string_field(e, "message");
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
            EventType::RepoMetricsCaptured => {
                // A point-in-time repo-metrics snapshot folded whole. The event
                // data IS the payload — deserialize defensively; a malformed
                // event is dropped rather than poisoning the projection.
                if let Ok(rm) = serde_json::from_value::<RepoMetrics>(e.data.clone()) {
                    self.repo_metrics.push(rm);
                }
            }
            EventType::MergeConflictDetected => {
                // A merge conflict is an observation-like event — it's recorded
                // in the event log but doesn't add a persistent entity to the
                // projection (the PM reacts to it, it doesn't become a board
                // item). It WILL wake the PM (Tier-1 trigger).
            }
            EventType::EntityArchived => {
                // An archived entity is added to the compact history and
                // excluded from the active lists. The full event log survives
                // for provenance queries.
                if let (Some(kind), Some(id)) =
                    (string_field(e, "entity_kind"), string_field(e, "entity_id"))
                {
                    // Remove from active lists
                    self.tasks.retain(|t| t.id != id);
                    self.decisions.retain(|d| d.id != id);
                    self.observations.retain(|o| o.id != id);
                    self.opinions.retain(|o| o.id != id);
                    self.risks.retain(|r| r.id != id);
                    // Add archival record
                    self.archived.push(crate::types::ArchivedRecord {
                        entity_kind: kind,
                        entity_id: id,
                        summary: string_field(e, "summary").unwrap_or_default(),
                        result: string_field(e, "result").unwrap_or_default(),
                        archived_at: e.timestamp.to_string(),
                        archived_by: string_field(e, "archived_by").unwrap_or_else(|| "system".into()),
                    });
                }
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
