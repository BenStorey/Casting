//! The PM's structured action vocabulary — the typed unitary actions and the
//! assignee model.
use crate::pm::{DecisionClass, OwnerInvolvement};
use crate::projection::Projection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The pseudo-assignee representing the HUMAN OWNER (the boss). When a task is
/// assigned to `"owner"`, the human — possibly working through their own
/// harness — executes and delivers it, rather than a hired agent. Distinct from
/// any agent id (roles are "lead-developer"/"testing-engineer"...; agents are "diego"...).
pub const OWNER: &str = "owner";

/// The reserved, NON-assignable special-role actors: the PM (co-ordinator) and
/// the Advisor (strategic thinking partner). They coordinate / advise / debate
/// but can NEVER be assigned implementation work — the PM cannot route a task
/// to itself, and the Advisor's conversations stay isolated from the project
/// event log. The policy gate (is_valid_assignee + HireAgent) treats these as
/// not-assignable and not-hirable, so an owner or model cannot accidentally
/// turn a special role into a task-doer.
pub const SPECIAL_ACTORS: &[&str] = &["pm", "advisor"];

/// True if `candidate` is a valid task assignee: either the human owner or a
/// hired agent — and never one of the reserved special roles. (owner
/// 2026-08-10 — human-as-consultant delivery.)
pub fn is_valid_assignee(state: &Projection, candidate: &str) -> bool {
    if candidate == OWNER || SPECIAL_ACTORS.contains(&candidate) {
        return candidate == OWNER;
    }
    state.agents.iter().any(|a| a.id == candidate)
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
    /// Assign a task to a hired agent. `merge_authority` is the PM's up-front
    /// decision on how the work merges (tiered merge policy, 2026-08-14):
    /// `self` = the assignee completes to Done directly after CI (trivial /
    /// peripheral); `pm` = the work must pass through the PM's review first.
    /// Recorded on the TaskAssigned event, so the decision is auditable.
    AssignTask {
        task_id: String,
        assignee: String,
        #[serde(default)]
        merge_authority: crate::types::MergeAuthority,
    },
    /// Reclassify a task's merge authority (the escape hatch when scope grows
    /// past its assignment label, e.g. a "trivial" change turned out to touch
    /// the core). PM/owner authority only. Records a `MergeAuthorityChanged`
    /// event so the merge decision stays auditable. `self -> pm` escalates the
    /// gate; `pm -> self` downgrades it (allowed, but a deliberate call).
    SetMergeAuthority {
        task_id: String,
        #[serde(default)]
        merge_authority: crate::types::MergeAuthority,
    },
    /// Provision an isolated worktree for a task (owner 2026-08-12): the
    /// platform gives a summoned consultant a dedicated working tree on its own
    /// branch with a private build target + distinct API port. This is the
    /// *structural* isolation guarantee — the agent is handed a ready workspace,
    /// never asked to "remember" to isolate. Produces WorktreeProvisioned.
    ProvisionWorktree {
        task_id: String,
        assignee: String,
        slug: String,
        cargo_target_dir: String,
        slot: usize,
        port: u16,
    },
    StartTask {
        task_id: String,
    },
    /// A consultant commits their work-in-progress into their isolated
    /// worktree's branch (2026-08-12). The thin agent git surface: the agent
    /// owns the CONTENT, the platform owns the isolation (the commit happens in
    /// the task's worktree via the pinned runner). Produces a commit + a
    /// ChangeSet commit record. The agent is never given a raw `git`.
    CommitToChangeSet {
        task_id: String,
        message: String,
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
        priority: crate::pm::plan::Priority,
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
        kind: crate::runtime::directive::DirectiveKind,
        statement: String,
        scope: Vec<String>,
        strength: crate::runtime::directive::DirectiveStrength,
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
        kind: crate::runtime::directive::DirectiveKind,
        statement: String,
        scope: Vec<String>,
        strength: crate::runtime::directive::DirectiveStrength,
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
    /// Decompose a task into child tasks (parallel-work fan-out). The parent is
    /// the join point: `Projection::graph()` aggregates all its children into
    /// the parent's resolution. Produces one `TaskDecomposed` + one
    /// `TaskCreated` per child (each carrying `parent_id`).
    DecomposeTask {
        parent: String,
        children: Vec<TaskSpec>,
    },
    /// Create a hard dependency edge: `task_id` cannot START until
    /// `blocking_task_id` reaches `required_state` (the Blocker Test's "No:
    /// hard dependency" outcome). Event-sourced — derived to
    /// `Projection.dependencies`. Soft/recommended-order notes stay Opinions.
    BlockTaskOn {
        task_id: String,
        blocking_task_id: String,
        required_state: crate::types::TaskStatus,
    },
    /// Explicitly conclude "nothing to do" (anti-thrash).
    NoOp,
    // --- Harness guards (2026-08-13) ---
    /// Owner sets the hard token budget (`POST /api/budget`). Only the owner may
    /// set it (it's the circuit breaker, outside PM control). `warn_at` is the
    /// fraction of `limit_usd` at which to warn (default 0.80).
    SetBudget {
        limit_usd: f64,
        #[serde(default)]
        warn_at: Option<f64>,
    },
    /// Pause all side-effecting work (owner action, or the liveness watchdog as
    /// system). Resumable via `ResumeWork`.
    PauseWork {
        reason: String,
    },
    /// Clear a `WorkPaused` (owner action). A BUDGET halt is NOT resumable by
    /// this — spend doesn't decrease; only a higher budget limit un-halts it.
    ResumeWork,
}

/// Specification for a child task created by `DecomposeTask`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    pub title: String,
    pub kind: String,
}

/// Generate the action vocabulary schema string for a given actor role.
///
/// Returns a structured string listing all actions the actor may perform,
/// with their JSON schema shapes. The provided actor string determines the
/// set of actions returned:
/// - `"pm"`, `"owner"`, or `"system"` — ALL actions (org, task, decisions,
///   knowledge, governance, comms, harness, and special).
/// - Any other actor (consultant) — only task, knowledge, communication,
///   and special actions they are permitted to perform.
pub fn action_vocab_for(actor: &str) -> String {
    let is_pm = matches!(actor, "pm" | "owner" | "system");

    // Helper: build a colon-separated entry like `field:type` from a name
    // and a type representation.
    let f = |name: &str, ty: &str| -> String { format!("  \"{name}\":{ty}") };

    let mut lines: Vec<String> = Vec::new();

    // ── ORGANISATIONAL ACTIONS (PM/owner only) ──────────────────────────
    if is_pm {
        lines.push("--- ORGANISATIONAL ACTIONS ---".into());
        lines.push(format!(
            "- hire_agent: {}{}{}",
            "{",
            [f("agent_id", "str"), f("role", "str")].join(","),
            "}"
        ));
        lines.push(format!(
            "- create_requirement: {}{}{}",
            "{",
            [f("id", "str"), f("title", "str"), f("description", "str")].join(","),
            "}"
        ));
        lines.push(format!(
            "- create_task: {}{}{}",
            "{",
            [f("id", "str"), f("title", "str"), f("kind", "str")].join(","),
            "}"
        ));
        lines.push(format!(
            "- assign_task: {}{}{}",
            "{",
            [
                f("task_id", "str"),
                f("assignee", "str"),
                f("merge_authority", "\"self\"|\"pm\""),
            ]
            .join(","),
            "}"
        ));
        lines.push(format!(
            "- set_merge_authority: {}{}{}",
            "{",
            [f("task_id", "str"), f("merge_authority", "\"self\"|\"pm\"")].join(","),
            "}"
        ));
        lines.push(format!(
            "- decompose_task: {}{}{}",
            "{",
            [
                f("parent", "str"),
                f(
                    "children",
                    "[{\"id\":str,\"title\":str,\"kind\":str}]",
                ),
            ]
            .join(","),
            "}"
        ));
        lines.push(format!(
            "- block_task_on: {}{}{}",
            "{",
            [
                f("task_id", "str"),
                f("blocking_task_id", "str"),
                f(
                    "required_state",
                    "\"backlog\"|\"working\"|\"in_review\"|\"blocked\"|\"done\"",
                ),
            ]
            .join(","),
            "}"
        ));
        lines.push(format!(
            "- provision_worktree: {}{}{}",
            "{",
            [
                f("task_id", "str"),
                f("assignee", "str"),
                f("slug", "str"),
                f("cargo_target_dir", "str"),
                f("slot", "0"),
                f("port", "u16"),
            ]
            .join(","),
            "}"
        ));
    }

    // ── TASK ACTIONS ────────────────────────────────────────────────────
    lines.push("--- TASK ACTIONS ---".into());
    lines.push(format!(
        "- start_task: {}{}{}",
        "{",
        [f("task_id", "str")].join(","),
        "}"
    ));
    lines.push(format!(
        "- complete_task: {}{}{}",
        "{",
        [f("task_id", "str"), f("result", "str")].join(","),
        "}"
    ));
    lines.push(format!(
        "- request_review: {}{}{}",
        "{",
        [f("task_id", "str"), f("reviewer", "str")].join(","),
        "}"
    ));
    lines.push(format!(
        "- review_task: {}{}{}",
        "{",
        [
            f("task_id", "str"),
            f("approved", "bool"),
            f("note", "str|null"),
        ]
        .join(","),
        "}"
    ));
    lines.push(format!(
        "- block_task: {}{}{}",
        "{",
        [f("task_id", "str"), f("reason", "str")].join(","),
        "}"
    ));
    lines.push(format!(
        "- commit_to_change_set: {}{}{}",
        "{",
        [f("task_id", "str"), f("message", "str")].join(","),
        "}"
    ));
    if is_pm {
        lines.push(format!(
            "- set_task_priority: {}{}{}",
            "{",
            [
                f("task_id", "str"),
                f("priority", "\"low\"|\"medium\"|\"high\"|\"critical\""),
            ]
            .join(","),
            "}"
        ));
    }

    // ── DECISIONS (PM/owner only) ────────────────────────────────────────
    if is_pm {
        lines.push("--- DECISIONS ---".into());
        lines.push(format!(
            "- propose_decision: {}{}{}",
            "{",
            [
                f("id", "str"),
                f("subject", "str"),
                f("options", "{...}"),
                f("recommendation", "str"),
                f("class",
                    "\"internal_implementation\"|\"internal_refactor\"|\"add_consultant\"|\"testing_library\"|\"security_critical\"|\"production_deployment\"|\"product_requirement\"|\"governance_change\"|\"database\"|\"internal_rename\"|\"architecture\"|\"spending_threshold\"|\"irreversible\""),
                f("involvement", "\"pm\"|\"ask\"|\"never\"|\"notify\""),
            ]
            .join(","),
            "}"
        ));
        lines.push(format!(
            "- make_decision: {}{}{}",
            "{",
            [
                f("decision_id", "str"),
                f("approved", "bool"),
                f("note", "str|null"),
            ]
            .join(","),
            "}"
        ));
        lines.push(format!(
            "- supersede_decision: {}{}{}",
            "{",
            [f("decision_id", "str"), f("by_decision_id", "str")].join(","),
            "}"
        ));
        lines.push(format!(
            "- propose_consultant: {}{}{}",
            "{",
            [
                f("id", "str"),
                f("subject", "str"),
                f("role_id", "str"),
                f("involvement", "\"pm\"|\"ask\"|\"never\""),
            ]
            .join(","),
            "}"
        ));
    }

    // ── KNOWLEDGE ───────────────────────────────────────────────────────
    lines.push("--- KNOWLEDGE ---".into());
    lines.push(format!(
        "- record_opinion: {}{}{}",
        "{",
        [
            f("id", "str"),
            f("subject", "str"),
            f("category", "str"),
            f("statement", "str"),
            f("supersedes", "str|null"),
        ]
        .join(","),
        "}"
    ));
    if is_pm {
        lines.push(format!(
            "- supersede_opinion: {}{}{}",
            "{",
            [f("opinion_id", "str"), f("by_opinion_id", "str")].join(","),
            "}"
        ));
    }
    lines.push(format!(
        "- record_fact: {}{}{}",
        "{",
        [f("id", "str"), f("kind", "str"), f("statement", "str")].join(","),
        "}"
    ));
    lines.push(format!(
        "- record_assumption: {}{}{}",
        "{",
        [f("id", "str"), f("body", "str")].join(","),
        "}"
    ));
    lines.push(format!(
        "- record_constraint: {}{}{}",
        "{",
        [f("id", "str"), f("body", "str")].join(","),
        "}"
    ));
    lines.push(format!(
        "- raise_risk: {}{}{}",
        "{",
        [f("id", "str"), f("subject", "str"), f("severity", "str")].join(","),
        "}"
    ));
    lines.push(format!(
        "- resolve_risk: {}{}{}",
        "{",
        [
            f("risk_id", "str"),
            f("status", "\"open\"|\"materialized\"|\"resolved\""),
        ]
        .join(","),
        "}"
    ));
    lines.push(format!(
        "- create_observation: {}{}{}",
        "{",
        [
            f("id", "str"),
            f("severity", "str"),
            f("subject", "str"),
            f("body", "str"),
            f("pm_action_required", "bool"),
        ]
        .join(","),
        "}"
    ));

    // ── GOVERNANCE (PM/owner only) ──────────────────────────────────────
    if is_pm {
        lines.push("--- GOVERNANCE ---".into());
        lines.push(format!(
            "- create_directive: {}{}{}",
            "{",
            [
                f("id", "str"),
                f("kind",
                    "\"policy\"|\"constraint\"|\"principle\"|\"practice\"|\"preference\"|\"objective\""),
                f("statement", "str"),
                f("scope", "[str]"),
                f("strength", "\"recommended\"|\"strong\"|\"required\""),
                f("supersedes", "str|null"),
            ]
            .join(","),
            "}"
        ));
        lines.push(format!(
            "- suspend_directive: {}{}{}",
            "{",
            [f("directive_id", "str")].join(","),
            "}"
        ));
        lines.push(format!(
            "- resume_directive: {}{}{}",
            "{",
            [f("directive_id", "str")].join(","),
            "}"
        ));
        lines.push(format!(
            "- supersede_directive: {}{}{}",
            "{",
            [
                f("directive_id", "str"),
                f("by_directive_id", "str"),
            ]
            .join(","),
            "}"
        ));
        lines.push(format!(
            "- expire_directive: {}{}{}",
            "{",
            [f("directive_id", "str")].join(","),
            "}"
        ));
        lines.push(format!(
            "- propose_directive_change: {}{}{}",
            "{",
            [
                f("id", "str"),
                f("subject", "str"),
                f("kind",
                    "\"policy\"|\"constraint\"|\"principle\"|\"practice\"|\"preference\"|\"objective\""),
                f("statement", "str"),
                f("scope", "[str]"),
                f("strength", "\"recommended\"|\"strong\"|\"required\""),
                f("supersedes", "str|null"),
            ]
            .join(","),
            "}"
        ));
    }

    // ── COMMUNICATION ──────────────────────────────────────────────────
    lines.push("--- COMMUNICATION ---".into());
    lines.push(format!(
        "- send_message: {}{}{}",
        "{",
        [f("to", "str"), f("body", "str")].join(","),
        "}"
    ));
    if is_pm {
        lines.push(format!(
            "- import_briefing: {}{}{}",
            "{",
            [
                f("id", "str"),
                f("source", "str"),
                f("subject", "str"),
                f("title", "str"),
                f("body", "str"),
                f("assets", "[{\"caption\":str,\"location\":str}]"),
            ]
            .join(","),
            "}"
        ));
        lines.push(format!(
            "- receive_external_request: {}{}{}",
            "{",
            [
                f("id", "str"),
                f("source", "str"),
                f("external_id", "str|null"),
                f("title", "str"),
                f("body", "str"),
                f("reporter", "str"),
                f("labels", "[str]"),
                f("url", "str|null"),
            ]
            .join(","),
            "}"
        ));
        lines.push(format!(
            "- save_diagram: {}{}{}",
            "{",
            [f("id", "str"), f("title", "str"), f("data", "str")].join(","),
            "}"
        ));
    }

    // ── SPECIAL ──────────────────────────────────────────────────────────
    lines.push("--- SPECIAL ---".into());
    lines.push(format!("- no_op: {}{}", "{", "}"));

    // ── HARNESS GUARDS (PM/owner only) ─────────────────────────────────
    if is_pm {
        lines.push("--- HARNESS ---".into());
        lines.push(format!(
            "- set_budget: {}{}{}",
            "{",
            [
                f("limit_usd", "f64"),
                f("warn_at", "f64|null"),
            ]
            .join(","),
            "}"
        ));
        lines.push(format!(
            "- pause_work: {}{}{}",
            "{",
            [f("reason", "str")].join(","),
            "}"
        ));
        lines.push(format!("- resume_work: {}{}", "{", "}"));
    }

    lines.join("\n")
}
