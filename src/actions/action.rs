//! The PM's structured action vocabulary — the typed unitary actions and the
//! assignee model.
use crate::policy::{DecisionClass, OwnerInvolvement};
use crate::projection::Projection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The pseudo-assignee representing the HUMAN OWNER (the boss). When a task is
/// assigned to `"owner"`, the human — possibly working through their own
/// harness — executes and delivers it, rather than a hired agent. Distinct from
/// any agent id (roles are "engineer"/"qa"...; agents are "marcus-reed"...).
pub const OWNER: &str = "owner";

/// True if `candidate` is a valid task assignee: either the human owner or a
/// hired agent. (owner 2026-08-10 — human-as-consultant delivery.)
pub fn is_valid_assignee(state: &Projection, candidate: &str) -> bool {
    candidate == OWNER || state.agents.iter().any(|a| a.id == candidate)
}

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
    /// Provision an isolated worktree for a task (owner 2026-08-12): the
    /// platform gives a summoned consultant a dedicated working tree on its own
    /// branch with a private build target + distinct API port. This is the
    /// *structural* isolation guarantee — the agent is handed a ready workspace,
    /// never asked to "remember" to isolate. Produces WorktreeProvisioned.
    ProvisionWorktree {
        task_id: String,
        slug: String,
        cargo_target_dir: String,
        port: u16,
    },
    StartTask {
        task_id: String,
    },
    CompleteTask {
        task_id: String,
        result: String,
    },
    /// Submit finished work for review. Produces TaskReadyForReview
    /// (status -> InReview). The reviewer is who later rules via ReviewTask.
    RequestReview {
        task_id: String,
        reviewer: String,
    },
    /// A reviewer rules on a task in review. Produces TaskReviewed: approved
    /// -> Done (review recorded) or rejected -> Working (rework).
    ReviewTask {
        task_id: String,
        approved: bool,
        note: Option<String>,
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
    /// Record a project OPINION — a subjective judgment / rationale /
    /// preference (e.g. "Postgres is a good default for our event log").
    RecordOpinion {
        id: String,
        subject: String,
        category: String,
        statement: String,
        /// Optional id of a prior opinion this one supersedes (not edited).
        supersedes: Option<String>,
    },
    /// Record a project FACT — an objective, measured point-in-time datapoint
    /// (e.g. "the repo is 1,342 lines").
    RecordFact {
        id: String,
        kind: String,
        statement: String,
    },
    /// Explicitly supersede a previously-recorded opinion (it is never edited;
    /// this sets its status to Superseded and preserves history). Mirrors
    /// SupersedeDirective. `by_opinion_id` names the newer opinion.
    SupersedeOpinion {
        opinion_id: String,
        by_opinion_id: String,
    },
    /// Import external advisor content (text + optional image/diagram refs) as
    /// an ADVISORY briefing — never authoritative (owner 2026-08-10).
    ImportBriefing {
        id: String,
        source: String,
        subject: String,
        title: String,
        body: String,
        assets: Vec<crate::projection::BriefingAsset>,
    },
    /// Receive an EXTERNAL request (e.g. a GitHub issue/PR) — the product's
    /// intake surface. Recorded with provenance + deterministic triage, NEVER
    /// as authoritative owner intent. (owner 2026-08-10)
    ReceiveExternalRequest {
        id: String,
        source: String,
        external_id: Option<String>,
        title: String,
        body: String,
        reporter: String,
        labels: Vec<String>,
        url: Option<String>,
    },
    /// Save a diagram drawn in the app (Excalidraw) as a durable visual artifact.
    /// `data` is the serialized Excalidraw JSON captured DIRECTLY from the editor.
    SaveDiagram {
        id: String,
        title: String,
        data: String,
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
    /// The PM proposes bringing a new consultant into the cast. Routes through
    /// the AddConsultant decision class: if the policy routes it to Pm, the PM
    /// auto-decides and hires; if the owner escalated it to Ask, it surfaces to
    /// the owner's inbox and is applied on approval.
    ProposeConsultant {
        id: String,
        subject: String,
        role_id: String,
        /// Resolved owner-involvement for the AddConsultant class (from policy).
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
