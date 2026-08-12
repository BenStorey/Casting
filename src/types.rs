//! Data types carried by the current-state projection.
//!
//! These are the small, derived projection entities (agents, tasks, decisions,
//! messages, risks, opinions, facts, briefings, external requests, diagrams,
//! git artifacts, ...). They are split out of `projection.rs` into their own
//! module; `projection.rs` re-exports them so `crate::projection::*` keeps
//! resolving for callers and tests.

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

/// Lifecycle of an external advisor briefing (owner 2026-08-10). Mirrors
/// directive/opinion supersession: active until a newer briefing supersedes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BriefingStatus {
    /// This briefing is currently being considered (still advisory, never
    /// authoritative — see `Briefing` docs).
    #[default]
    Active,
    /// Replaced by a newer briefing (history preserved; it no longer shapes the
    /// operating context at full weight).
    Superseded,
}

/// An EXTERNAL advisor briefing imported into the project (e.g. a plan from a
/// ChatGPT conversation pasted in by the owner). It is deliberately **advisory,
/// NOT authoritative**: `source` records where it came from so it's never
/// confusable with the owner's own intent, and it can *inform* context but NEVER
/// sets rules (directives remain the only way to assert authority). Scoped by
/// `subject` so it can be looked up ("what does my advisor say about X?") rather
/// than dominating all context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Briefing {
    pub id: String,
    /// Where this came from, e.g. "ChatGPT advisor", "email from Ben", "paste".
    pub source: String,
    /// The topic this advice is about, e.g. "storage" | "architecture".
    pub subject: String,
    /// A short human label, e.g. "chatgpt D2 plan".
    pub title: String,
    /// The advisory text body.
    pub body: String,
    /// Reference handles for images/diagrams (paths/URLs + caption). Content is
    /// NOT embedded in the event; a future vision pass can derive from them.
    #[serde(default)]
    pub assets: Vec<BriefingAsset>,
    pub brought_in_by: String,
    pub status: BriefingStatus,
    /// If non-empty, the id of a prior briefing this supersedes.
    #[serde(default)]
    pub supersedes: Option<String>,
    pub imported_at: String,
}

/// A reference to an image/diagram that accompanied an advisor briefing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefingAsset {
    pub caption: String,
    /// Path or URL to the asset.
    pub location: String,
}

/// A request raised from an EXTERNAL source (e.g. a GitHub issue/PR a product
/// user opened). This is the product's INTAKE surface — the analog of a
/// Requirement (owner message) and an AdvisoryBriefing (advisor import) but for
/// "what a user reported". Deliberately NOT authoritative: it's a request from
/// outside, recorded with provenance (`source`, `external_id`, `reporter`) so
/// the PM can triage it without it pretending to be the owner's own intent.
/// Deterministic triage (classification/priority) is a projection concern; the
/// LLM later decides whether to act on it (docs/HARNESS.md, D2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExternalRequest {
    pub id: String,
    /// Where the request came from, e.g. "github".
    pub source: String,
    /// The id in the external system (e.g. GitHub issue #42), if any.
    #[serde(default)]
    pub external_id: Option<String>,
    pub title: String,
    pub body: String,
    /// Who raised it (e.g. GitHub username).
    pub reporter: String,
    /// Labels/tags from the source (e.g. ["bug", "security"]).
    #[serde(default)]
    pub labels: Vec<String>,
    /// The source URL (e.g. the GitHub issue link), if any.
    #[serde(default)]
    pub url: Option<String>,
    /// Deterministic classification (triage): "bug" | "feature" | "other".
    pub classification: String,
    /// Deterministic severity estimate: low | medium | high.
    pub severity: String,
    /// Whether the PM has acted on it yet (open) or closed it.
    pub status: ExternalRequestStatus,
    pub received_at: String,
}

/// Lifecycle of an external request (intake surface; see ExternalRequest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExternalRequestStatus {
    /// Received from outside; not yet triaged/acted on.
    #[default]
    Open,
    /// Triaged and closed (decided no action, duplicate, resolved upstream).
    Closed,
}

/// A diagram drawn and saved inside the app (Excalidraw canvas). A durable visual
/// artifact — the serialized Excalidraw document (`data`) captured DIRECTLY from
/// the editor at save time (owner 2026-08-10), so there's no export/re-upload.
/// The PM/owner can view and reload it. It is a documented artifact (like a
/// briefing asset), not authoritative state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagram {
    pub id: String,
    /// Human title the owner gave the diagram, e.g. "Auth flow sketch".
    pub title: String,
    /// Serialized Excalidraw JSON (the full returned doc) — reloadable into Excalidraw.
    pub data: String,
    /// Who saved it (owner/agent id/system).
    pub saved_by: String,
    pub saved_at: String,
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

/// An isolated worktree provisioned for a summoned consultant (owner,
/// 2026-08-12). The "consultant's desk": a dedicated working tree on its own
/// branch, with a private build target and a distinct API port so concurrent
/// consultants cannot collide. Isolation is a PLATFORM property — the platform
/// provisions it, the agent just works here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Worktree {
    /// The task this workspace serves.
    pub task_id: String,
    /// The worktree's own branch (casting/task-<id>-<slug>).
    pub branch: String,
    /// Filesystem path to the worktree (under <repo>/.casting/worktrees/).
    pub path: String,
    /// Private CARGO_TARGET_DIR so concurrent builds don't stomp each other.
    pub cargo_target_dir: String,
    /// Distinct API port so concurrent dev servers can run in parallel.
    pub port: u16,
}
