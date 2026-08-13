//! Typed domain events — the append-only history is the source of truth.
//!
//! Mirrors the event anatomy in docs/CASTING_PROJECT_BRIEF.md §11 and the
//! domain-vs-runtime separation in §12. Only DOMAIN events belong here;
//! low-level telemetry (token streams, shell commands, git plumbing) lives
//! elsewhere and never enters this store's history.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Who or what caused an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    /// The human owner.
    Owner,
    /// An agent, identified by its stable id (e.g. "marcus-reed", "pm").
    Agent { id: String },
    /// The system itself (e.g. background watchers, persistence).
    System,
}

/// Semantic type of a domain event. Deliberately a small, curated set —
/// the organizational model (docs/CASTING_PROJECT_BRIEF.md §12) rather
/// than raw machinery. Git semantic events per docs/ADDENDUM.md §23.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    // --- Organizational events ---
    ProjectCreated,
    AgentHired,
    RequirementCreated,
    RequirementChanged,
    TaskCreated,
    TaskAssigned,
    TaskStarted,
    TaskCompleted,
    TaskBlocked,
    /// Work is submitted for review; status -> InReview.
    TaskReadyForReview,
    /// A reviewer ruled on a task: approved (-> Done, review recorded) or
    /// rejected (-> Working, rework requested).
    TaskReviewed,
    /// A task's priority changed (per docs/SEMANTIC_EVENTS.md: a mutation; the
    /// projection reduces it to `task.priority` deterministically).
    TaskPriorityChanged,
    /// A task was decomposed into child tasks (parallel-work fan-out). The
    /// parent is the join point — the graph aggregates its children's
    /// resolution into the parent. Records the decomposition *intent* for
    /// provenance (`{ parent, children: [...] }`); each child also arrives via
    /// its own `TaskCreated` carrying `parent_id`.
    TaskDecomposed,
    /// A hard dependency edge: `task` (aggregate id) cannot START until
    /// `blocking_task` reaches `required_state`. The Blocker Test's "No: hard
    /// dependency" outcome. Event-sourced — derived to `Projection.dependencies`
    /// (NEVER a side table). Soft/recommended-order notes are Opinions, not this.
    TaskBlockedOn,
    /// A risk was raised (a first-class semantic object, SEMANTIC_EVENTS §8).
    RiskRaised,
    /// A risk's status changed (resolved / materialized).
    RiskUpdated,
    /// A project assumption was recorded (semantic object).
    AssumptionRecorded,
    /// A project constraint was recorded (semantic object).
    ConstraintRecorded,
    /// A project OPINION was recorded — a subjective judgment / rationale /
    /// preference (e.g. "Postgres is a good default for our log"). Subjective
    /// and changeable: superseded, never edited. (owner concept 2026-08-10:
    /// knowledge worth not re-deriving is opinion, not objective fact)
    OpinionRecorded,
    /// A previously-recorded opinion is superseded by a newer one -> status
    /// Superseded (history preserved). Mirrors ProjectDirectiveSuperseded.
    OpinionSuperseded,
    /// A project FACT was recorded — an objective, measured point-in-time datapoint
    /// (e.g. "the repo is 1,342 lines"). Objective measures are usually
    /// derived from state, but recording one captures a point-in-time snapshot
    /// worth preserving. (owner concept 2026-08-10)
    FactRecorded,
    /// Cost incurred by an agent/model call (harness responsibility #6 — cost
    /// attribution & token budgeting, docs/HARNESS.md). Carries provider
    /// metering so spend is attributable per agent/task from the event log,
    /// not tracked separately. The PM's "budget concern" reads this projection.
    CostIncurred,
    /// A project directive was created (governance layer, docs/INTENT.md).
    ProjectDirectiveCreated,
    /// A directive was suspended (no longer governs) — status -> Suspended.
    ProjectDirectiveSuspended,
    /// A directive was resumed (governs again) — status -> Active.
    ProjectDirectiveResumed,
    /// A directive was replaced by another (history preserved) -> Superseded.
    ProjectDirectiveSuperseded,
    /// A directive expired -> Expired.
    ProjectDirectiveExpired,
    ObservationCreated,
    DecisionProposed,
    /// A decision was resolved — by the OWNER (after being asked) OR by a
    /// delegated PM/agent. This is the universal decision-maker event: there
    /// is no separate "owner decision" type; the actor on this event is who
    /// decided (docs/CASTING_PROJECT_BRIEF.md §5, HANDOFF decision log).
    DecisionMade,
    /// A decision was superseded by another (history preserved, never deleted).
    /// Status -> Superseded; `superseded_by` links to the replacing decision
    /// (docs/SEMANTIC_EVENTS.md §22).
    DecisionSuperseded,
    /// The owner set/changed the owner-involvement required for a decision
    /// class (delegated authority, brief §5). Event-sourced so the autonomy
    /// configuration is durable history, not a hardcoded default.
    DecisionPolicyChanged,
    MessageSent,

    // --- Semantic Git events (ADDENDUM §23) ---
    /// A new branch appeared in the repo (typically `casting/task-N-*`).
    BranchCreated,
    /// A new commit was observed on a branch.
    CommitObserved,
    /// A merge was completed (a merge commit appeared on a protected branch).
    MergeCompleted,
    /// A merge attempt resulted in conflicts (emitted by the git runner, not
    /// the passive observer — requires attempting the merge).
    MergeConflictDetected,
    /// A batch of work is ready for review (task + branch + commits assembled
    /// into a ChangeSet). Emitted by the ChangeSet layer (increment 3).
    ChangeSetReady,
    /// An isolated worktree was provisioned for a task (owner 2026-08-12): the
    /// platform gave a summoned consultant a dedicated working tree on its own
    /// branch, with a private build target and a distinct API port. Carries the
    /// task_id, branch, path, cargo_target_dir, and port. Isolation is a PLATFORM
    /// property, not an agent behavior — the observer no longer has to guess
    /// task_id from the branch name; the mapping is recorded exactly.
    WorktreeProvisioned,
    /// A consultant explicitly asked to commit their WIP into their worktree
    /// (2026-08-12). Provenance for the agent git surface — the actual commit
    /// lands via the git runner and is also recorded as a CommitObserved by the
    /// observer; this event captures the *intent* ("the assignee decided to
    /// checkpoint here").
    CommitRequested,
    /// A worktree was pruned once its task completed/merged (2026-08-12).
    /// Removes the Worktree from the projection (freeing its port for reuse)
    /// and the physical tree via the reconciler. The WorktreeProvisioned event
    /// remains as history — this is the lifecycle close.
    WorktreeRemoved,

    // --- External advisory context (owner, 2026-08-10) ---
    /// Advisor content (text and/or image/diagram references) brought INTO the
    /// project from OUTSIDE Casting (e.g. a ChatGPT plan Ben pastes in). It is
    /// explicitly **advisory, NOT authoritative** — it can inform context but
    /// NEVER sets rules. Carries provenance (`source`) so it's never confusable
    /// with the owner's own intent. Supersedable (`status`/`supersedes`), so
    /// stale advice decays instead of dominating context forever.
    AdvisoryBriefingImported,
    /// A request arrived from an EXTERNAL source (e.g. a GitHub issue/PR a
    /// product user opened). The product's intake surface. Carries provenance
    /// (source, external_id, reporter) so the PM can triage it; NOT the owner's
    /// own intent. (owner 2026-08-10)
    ExternalRequestReceived,
    /// A diagram was drawn and saved inside the app (Excalidraw canvas) — a durable
    /// visual artifact captured directly from the editor, not a re-uploaded
    /// image. Stored as serialized Excalidraw JSON (`data`) the PM/owner can view
    /// and reload. (owner 2026-08-10)
    DiagramSaved,
    /// A message in the owner↔advisor thread — the owner's private chat with the
    /// direction-advisor (a special second role the owner interacts with
    /// directly). This thread is ISOLATED from the PM's context by design: it
    /// only reaches the PM when the owner explicitly hands it off via
    /// `AdvisorHandoff` (which becomes an AdvisoryBriefing). (owner 2026-08-10)
    AdvisorMessageSent,
    /// The owner asked the advisor thread to be summarized and handed off to the
    /// PM — converting the private strategic conversation into an
    /// `AdvisoryBriefing` (provenanced "advisor") the PM DOES read. This is the
    /// explicit integration point between the two direct owner roles.
    AdvisorHandoff,
    // --- Durable execution (durability first PR, docs/plans/2026-08-13) ---
    /// Intent to run a discrete side-effecting action (an LLM call, a git push,
    /// a shell command). Carries a STABLE `activity.id` (the idempotency key,
    /// e.g. `task-7-llm-call-3`) + the full serialized `Activity` so a
    /// crash-triggered re-dispatch can reconstruct it. The durable record that
    /// execution was planned and may need re-running after a crash.
    ActivityScheduled,
    /// The executor finished the activity: `{ id, result_ref }`. This is the
    /// idempotency MARKER — once present, `execute` skips the activity.
    ActivityCompleted,
    /// The executor's activity errored: `{ id, error }`. Feeds a retry DECISION
    /// (PM layer), never a machinery-side retry counter.
    ActivityFailed,
    // --- Harness guards (2026-08-13, docs/plans/2026-08-13_harness-guards.md) ---
    /// The owner set a hard token budget: `{ limit_usd, warn_at }`. Folds into
    /// `proj.budget`; the dispatch gate refuses LLM calls once spend >= limit.
    BudgetSet,
    /// A resumable pause of all side-effecting work: `{ reason, by }`. Cleared
    /// by `WorkResumed`. (The budget halt is DERIVED from spend, not this event.)
    WorkPaused,
    /// The owner (or a guard clearing its own pause) resumes work.
    WorkResumed,
    // --- Diagnostics / audit trail (2026-08) ---
    /// A proposed PM action (from the scripted plans OR the D2 orchestrator /
    /// real LLM) was refused by the policy gate. Records WHO proposed it, WHAT
    /// action, and WHY it was rejected — so a misbehaving model's plan is
    /// auditable instead of being silently dropped to a server log.
    PlanActionRejected,
    /// An orchestrator planning pass was recorded: what context was handed in,
    /// what actions the orchestrator returned, and the call's metering. The
    /// "what did the model see & decide on this trigger" trace for testing the
    /// LLM seam end-to-end.
    OrchestrationRun,
}

/// The entity primarily affected by an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aggregate {
    pub kind: String,
    pub id: String,
}

/// Correlation/causation links (docs/CASTING_PROJECT_BRIEF.md §11).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// The larger operation this event belongs to.
    pub correlation_id: Option<String>,
    /// The event that directly caused this one.
    pub causation_id: Option<Uuid>,
    /// The underlying agent/model execution, if any.
    pub agent_run_id: Option<String>,
}

/// One immutable, append-only domain event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Globally unique id.
    pub event_id: Uuid,
    pub project_id: String,
    /// Monotonically increasing sequence within the project. Authoritative
    /// ordering — do not rely on timestamps alone.
    pub sequence: i64,
    pub timestamp: DateTime<Utc>,
    pub actor: Actor,
    pub event_type: EventType,
    pub aggregate: Aggregate,
    /// Event-specific structured payload (JSON-compatible).
    pub data: serde_json::Value,
    pub metadata: Metadata,
}

impl Event {
    /// Build a new event with a fresh id/timestamp and the supplied fields.
    /// `sequence` is supplied by the store on append, not here.
    pub fn new(
        project_id: impl Into<String>,
        actor: Actor,
        event_type: EventType,
        aggregate: Aggregate,
        data: serde_json::Value,
    ) -> Self {
        Event {
            event_id: Uuid::new_v4(),
            project_id: project_id.into(),
            sequence: 0, // assigned on append
            timestamp: Utc::now(),
            actor,
            event_type,
            aggregate,
            data,
            metadata: Metadata::default(),
        }
    }
}
