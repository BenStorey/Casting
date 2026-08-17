//! The PM's structured action vocabulary — the typed unitary actions and the
//! assignee model.
use crate::pm::{DecisionClass, OwnerInvolvement};
use crate::projection::Projection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The pseudo-assignee representing a DIRECTOR of the company. When a task is
/// assigned to `"director"`, the human — possibly working through their own
/// harness — executes and delivers it, rather than a hired agent. Named
/// "director" (not "director") because there may be more than one; for day 1
/// there is only the CEO.
pub const DIRECTOR: &str = "director";

/// The reserved, NON-assignable special-role actors: the PM (co-ordinator) and
/// the Advisor (strategic thinking partner). They coordinate / advise / debate
/// but can NEVER be assigned implementation work — the PM cannot route a task
/// to itself, and the Advisor's conversations stay isolated from the project
/// event log. The policy gate (is_valid_assignee + HireAgent) treats these as
/// not-assignable and not-hirable, so a director or model cannot accidentally
/// turn a special role into a task-doer.
///
/// NOTE: "pm" is NOT in this list because the PM may self-assign tasks
/// through the `chat-interface` playbook for small direct work. The
/// policy gate has a dedicated carve-out for PM self-assignment; "advisor"
/// and future non-implementer roles stay blocked.
pub const SPECIAL_ACTORS: &[&str] = &["advisor"];

/// True if `candidate` is a valid task assignee: either the human director, the
/// PM (for self-assigned small work via the chat-interface playbook), or a
/// hired agent — and never one of the other reserved special roles. (director
/// 2026-08-10 — human-as-consultant delivery.)
pub fn is_valid_assignee(state: &Projection, candidate: &str) -> bool {
    if candidate == DIRECTOR || candidate == "pm" {
        return true;
    }
    if SPECIAL_ACTORS.contains(&candidate) {
        return false;
    }
    state.agents.iter().any(|a| a.id == candidate)
}

/// One organizational move the PM may propose. Serde-tagged so an LLM can emit
/// it as JSON and it round-trips 1:1 with what a scripted policy builds.
///
/// Each variant carries exactly the fields needed to execute the action; the
/// aggregate id of the resulting event is the action's entity id. Several map
/// to a single domain event; a few span two (e.g. proposing a decision also
/// results in a message to the director).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PmAction {
    /// Bring a consultant into the company.
    HireAgent {
        agent_id: String,
        role: String,
    },
    /// Record a requirement derived from director intent.
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
    /// the core). PM/director authority only. Records a `MergeAuthorityChanged`
    /// event so the merge decision stays auditable. `self -> pm` escalates the
    /// gate; `pm -> self` downgrades it (allowed, but a deliberate call).
    SetMergeAuthority {
        task_id: String,
        #[serde(default)]
        merge_authority: crate::types::MergeAuthority,
    },
    /// Provision an isolated worktree for a task (director 2026-08-12): the
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
    /// an ADVISORY briefing — never authoritative (director 2026-08-10).
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
    /// as authoritative director intent. (director 2026-08-10)
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
    /// decision, which routes to the director (Ask); on approval the change is
    /// applied on the director's authority.
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
    /// Ask the director to rule on a decision (delegated authority).
    ProposeDecision {
        id: String,
        subject: String,
        options: Value,
        recommendation: String,
        /// The decision's class — drives which director involvement the policy
        /// engine requires (and thus who the decision-maker is).
        class: DecisionClass,
        /// The resolved director involvement claimed by the producer. `validate`
        /// rejects this if it undercuts what the policy requires for `class`
        /// (authority-downgrade guard).
        involvement: OwnerInvolvement,
    },
    /// The PM proposes bringing a new consultant into the cast. Routes through
    /// the AddConsultant decision class: if the policy routes it to Pm, the PM
    /// auto-decides and hires; if the director escalated it to Ask, it surfaces to
    /// the director's inbox and is applied on approval.
    ProposeConsultant {
        id: String,
        subject: String,
        role_id: String,
        /// Resolved director-involvement for the AddConsultant class (from policy).
        involvement: OwnerInvolvement,
    },
    /// Resolve a decision. The universal decision-maker step: the actor is who
    /// decided — `Owner` after being asked, or a delegated PM/agent (per policy).
    /// Produces `DecisionMade`; there is no separate director-decision event.
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
    /// A human-readable message to the director / another agent.
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
    /// Apply a named playbook from a consultant's catalog (or an ad-hoc
    /// inline recipe) to decompose a parent task into step-tasks. `version`
    /// is Some for packaged playbooks, None for ad-hoc. `recipe` is None
    /// when referencing a packaged playbook in the consultant's registry.
    ApplyPlaybook {
        playbook_id: String,
        parent_task_id: String,
        version: Option<u32>,
        recipe: Option<AdHocRecipe>,
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
    /// Owner sets the hard token budget (`POST /api/budget`). Only the director may
    /// set it (it's the circuit breaker, outside PM control). `warn_at` is the
    /// fraction of `limit_usd` at which to warn (default 0.80).
    SetBudget {
        limit_usd: f64,
        #[serde(default)]
        warn_at: Option<f64>,
    },
    /// Pause all side-effecting work (director action, or the liveness watchdog as
    /// system). Resumable via `ResumeWork`.
    PauseWork {
        reason: String,
    },
    /// Clear a `WorkPaused` (director action). A BUDGET halt is NOT resumable by
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

/// An ad-hoc recipe provided inline (not from the consultant's packaged
/// playbook catalog). The PM may author a one-off recipe at apply time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdHocRecipe {
    pub title: String,
    pub problem: String,
    pub cost_band: crate::consultants::playbook::CostBand,
    pub steps: Vec<crate::consultants::playbook::PlaybookStep>,
}

/// A single action vocabulary entry: its name, section header, PM-only flag,
/// and the JSON-ish schema string for its fields (e.g. `"task_id":str, "note":str|null`).
struct ActionVocabEntry {
    name: &'static str,
    section: &'static str,
    pm_only: bool,
    fields: &'static [(&'static str, &'static str)],
}

/// All known action entries, derived from the PmAction enum variants.
///
/// Each entry's field *names* MUST match the Rust struct field names of the
/// corresponding PmAction variant. The field *types* are LLM-facing schema
/// descriptors (e.g. `"str"`, `"bool"`, `"str|null"`, `"\\"self\\"|\\"pm\\""`).
///
/// Additions and removals here should mirror PmAction changes; the field-name
/// correspondence is checked at review time.
const ACTION_VOCAB: &[ActionVocabEntry] = &[
    // ── ORGANISATIONAL ACTIONS (PM/director only) ──────────────────────────
    ActionVocabEntry {
        name: "hire_agent",
        section: "--- ORGANISATIONAL ACTIONS ---",
        pm_only: true,
        fields: &[("agent_id", "str"), ("role", "str")],
    },
    ActionVocabEntry {
        name: "create_requirement",
        section: "--- ORGANISATIONAL ACTIONS ---",
        pm_only: true,
        fields: &[("id", "str"), ("title", "str"), ("description", "str")],
    },
    ActionVocabEntry {
        name: "create_task",
        section: "--- ORGANISATIONAL ACTIONS ---",
        pm_only: true,
        fields: &[("id", "str"), ("title", "str"), ("kind", "str")],
    },
    ActionVocabEntry {
        name: "assign_task",
        section: "--- ORGANISATIONAL ACTIONS ---",
        pm_only: true,
        fields: &[
            ("task_id", "str"),
            ("assignee", "str"),
            ("merge_authority", "\"self\"|\"pm\""),
        ],
    },
    ActionVocabEntry {
        name: "set_merge_authority",
        section: "--- ORGANISATIONAL ACTIONS ---",
        pm_only: true,
        fields: &[("task_id", "str"), ("merge_authority", "\"self\"|\"pm\"")],
    },
    ActionVocabEntry {
        name: "decompose_task",
        section: "--- ORGANISATIONAL ACTIONS ---",
        pm_only: true,
        fields: &[
            ("parent", "str"),
            ("children", "[{\"id\":str,\"title\":str,\"kind\":str}]"),
        ],
    },
    ActionVocabEntry {
        name: "block_task_on",
        section: "--- ORGANISATIONAL ACTIONS ---",
        pm_only: true,
        fields: &[
            ("task_id", "str"),
            ("blocking_task_id", "str"),
            (
                "required_state",
                "\"backlog\"|\"working\"|\"in_review\"|\"blocked\"|\"done\"",
            ),
        ],
    },
    ActionVocabEntry {
        name: "apply_playbook",
        section: "--- ORGANISATIONAL ACTIONS ---",
        pm_only: true,
        fields: &[
            ("playbook_id", "str"),
            ("parent_task_id", "str"),
            ("version", "u32|null"),
            ("recipe", "{\"title\":str,\"problem\":str,\"cost_band\":\"cheap\"|\"medium\"|\"expensive\",\"steps\":[{\"id\":str,\"title\":str,\"model\":str,\"prompt\":str,\"artifact\":str,\"produces\":str,\"reads\":[str]}]}|null"),
        ],
    },
    ActionVocabEntry {
        name: "provision_worktree",
        section: "--- ORGANISATIONAL ACTIONS ---",
        pm_only: true,
        fields: &[
            ("task_id", "str"),
            ("assignee", "str"),
            ("slug", "str"),
            ("cargo_target_dir", "str"),
            ("slot", "0"),
            ("port", "u16"),
        ],
    },
    // ── TASK ACTIONS ────────────────────────────────────────────────────
    ActionVocabEntry {
        name: "start_task",
        section: "--- TASK ACTIONS ---",
        pm_only: false,
        fields: &[("task_id", "str")],
    },
    ActionVocabEntry {
        name: "complete_task",
        section: "--- TASK ACTIONS ---",
        pm_only: false,
        fields: &[("task_id", "str"), ("result", "str")],
    },
    ActionVocabEntry {
        name: "request_review",
        section: "--- TASK ACTIONS ---",
        pm_only: false,
        fields: &[("task_id", "str"), ("reviewer", "str")],
    },
    ActionVocabEntry {
        name: "review_task",
        section: "--- TASK ACTIONS ---",
        pm_only: false,
        fields: &[
            ("task_id", "str"),
            ("approved", "bool"),
            ("note", "str|null"),
        ],
    },
    ActionVocabEntry {
        name: "block_task",
        section: "--- TASK ACTIONS ---",
        pm_only: false,
        fields: &[("task_id", "str"), ("reason", "str")],
    },
    ActionVocabEntry {
        name: "commit_to_change_set",
        section: "--- TASK ACTIONS ---",
        pm_only: false,
        fields: &[("task_id", "str"), ("message", "str")],
    },
    ActionVocabEntry {
        name: "set_task_priority",
        section: "--- TASK ACTIONS ---",
        pm_only: true,
        fields: &[
            ("task_id", "str"),
            ("priority", "\"low\"|\"medium\"|\"high\"|\"critical\""),
        ],
    },
    // ── DECISIONS (PM/director only) ────────────────────────────────────────
    ActionVocabEntry {
        name: "propose_decision",
        section: "--- DECISIONS ---",
        pm_only: true,
        fields: &[
            ("id", "str"),
            ("subject", "str"),
            ("options", "{...}"),
            ("recommendation", "str"),
            (
                "class",
                "\"internal_implementation\"|\"internal_refactor\"|\"add_consultant\"|\"testing_library\"|\"security_critical\"|\"production_deployment\"|\"product_requirement\"|\"governance_change\"|\"database\"|\"internal_rename\"|\"architecture\"|\"spending_threshold\"|\"irreversible\"",
            ),
            ("involvement", "\"pm\"|\"ask\"|\"never\"|\"notify\""),
        ],
    },
    ActionVocabEntry {
        name: "make_decision",
        section: "--- DECISIONS ---",
        pm_only: true,
        fields: &[
            ("decision_id", "str"),
            ("approved", "bool"),
            ("note", "str|null"),
        ],
    },
    ActionVocabEntry {
        name: "supersede_decision",
        section: "--- DECISIONS ---",
        pm_only: true,
        fields: &[("decision_id", "str"), ("by_decision_id", "str")],
    },
    ActionVocabEntry {
        name: "propose_consultant",
        section: "--- DECISIONS ---",
        pm_only: true,
        fields: &[
            ("id", "str"),
            ("subject", "str"),
            ("role_id", "str"),
            ("involvement", "\"pm\"|\"ask\"|\"never\""),
        ],
    },
    // ── KNOWLEDGE ───────────────────────────────────────────────────────
    ActionVocabEntry {
        name: "record_opinion",
        section: "--- KNOWLEDGE ---",
        pm_only: false,
        fields: &[
            ("id", "str"),
            ("subject", "str"),
            ("category", "str"),
            ("statement", "str"),
            ("supersedes", "str|null"),
        ],
    },
    ActionVocabEntry {
        name: "supersede_opinion",
        section: "--- KNOWLEDGE ---",
        pm_only: true,
        fields: &[("opinion_id", "str"), ("by_opinion_id", "str")],
    },
    ActionVocabEntry {
        name: "record_fact",
        section: "--- KNOWLEDGE ---",
        pm_only: false,
        fields: &[("id", "str"), ("kind", "str"), ("statement", "str")],
    },
    ActionVocabEntry {
        name: "record_assumption",
        section: "--- KNOWLEDGE ---",
        pm_only: false,
        fields: &[("id", "str"), ("body", "str")],
    },
    ActionVocabEntry {
        name: "record_constraint",
        section: "--- KNOWLEDGE ---",
        pm_only: false,
        fields: &[("id", "str"), ("body", "str")],
    },
    ActionVocabEntry {
        name: "raise_risk",
        section: "--- KNOWLEDGE ---",
        pm_only: false,
        fields: &[("id", "str"), ("subject", "str"), ("severity", "str")],
    },
    ActionVocabEntry {
        name: "resolve_risk",
        section: "--- KNOWLEDGE ---",
        pm_only: false,
        fields: &[
            ("risk_id", "str"),
            ("status", "\"open\"|\"materialized\"|\"resolved\""),
        ],
    },
    ActionVocabEntry {
        name: "create_observation",
        section: "--- KNOWLEDGE ---",
        pm_only: false,
        fields: &[
            ("id", "str"),
            ("severity", "str"),
            ("subject", "str"),
            ("body", "str"),
            ("pm_action_required", "bool"),
        ],
    },
    // ── GOVERNANCE (PM/director only) ──────────────────────────────────────
    ActionVocabEntry {
        name: "create_directive",
        section: "--- GOVERNANCE ---",
        pm_only: true,
        fields: &[
            ("id", "str"),
            (
                "kind",
                "\"policy\"|\"constraint\"|\"principle\"|\"practice\"|\"preference\"|\"objective\"",
            ),
            ("statement", "str"),
            ("scope", "[str]"),
            ("strength", "\"recommended\"|\"strong\"|\"required\""),
            ("supersedes", "str|null"),
        ],
    },
    ActionVocabEntry {
        name: "suspend_directive",
        section: "--- GOVERNANCE ---",
        pm_only: true,
        fields: &[("directive_id", "str")],
    },
    ActionVocabEntry {
        name: "resume_directive",
        section: "--- GOVERNANCE ---",
        pm_only: true,
        fields: &[("directive_id", "str")],
    },
    ActionVocabEntry {
        name: "supersede_directive",
        section: "--- GOVERNANCE ---",
        pm_only: true,
        fields: &[("directive_id", "str"), ("by_directive_id", "str")],
    },
    ActionVocabEntry {
        name: "expire_directive",
        section: "--- GOVERNANCE ---",
        pm_only: true,
        fields: &[("directive_id", "str")],
    },
    ActionVocabEntry {
        name: "propose_directive_change",
        section: "--- GOVERNANCE ---",
        pm_only: true,
        fields: &[
            ("id", "str"),
            ("subject", "str"),
            (
                "kind",
                "\"policy\"|\"constraint\"|\"principle\"|\"practice\"|\"preference\"|\"objective\"",
            ),
            ("statement", "str"),
            ("scope", "[str]"),
            ("strength", "\"recommended\"|\"strong\"|\"required\""),
            ("supersedes", "str|null"),
        ],
    },
    // ── COMMUNICATION ──────────────────────────────────────────────────
    ActionVocabEntry {
        name: "send_message",
        section: "--- COMMUNICATION ---",
        pm_only: false,
        fields: &[("to", "str"), ("body", "str")],
    },
    ActionVocabEntry {
        name: "import_briefing",
        section: "--- COMMUNICATION ---",
        pm_only: true,
        fields: &[
            ("id", "str"),
            ("source", "str"),
            ("subject", "str"),
            ("title", "str"),
            ("body", "str"),
            ("assets", "[{\"caption\":str,\"location\":str}]"),
        ],
    },
    ActionVocabEntry {
        name: "receive_external_request",
        section: "--- COMMUNICATION ---",
        pm_only: true,
        fields: &[
            ("id", "str"),
            ("source", "str"),
            ("external_id", "str|null"),
            ("title", "str"),
            ("body", "str"),
            ("reporter", "str"),
            ("labels", "[str]"),
            ("url", "str|null"),
        ],
    },
    ActionVocabEntry {
        name: "save_diagram",
        section: "--- COMMUNICATION ---",
        pm_only: true,
        fields: &[("id", "str"), ("title", "str"), ("data", "str")],
    },
    // ── SPECIAL ──────────────────────────────────────────────────────────
    ActionVocabEntry {
        name: "no_op",
        section: "--- SPECIAL ---",
        pm_only: false,
        fields: &[],
    },
];

/// Generate the action vocabulary schema string for a given actor role.
///
/// Returns a structured string listing all actions the actor may perform,
/// with their JSON schema shapes. The provided actor string determines the
/// set of actions returned:
/// - `"pm"`, `"director"`, or `"system"` — ALL actions (org, task, decisions,
///   knowledge, governance, comms, harness, and special).
/// - Any other actor (consultant) — only task, knowledge, communication,
///   and special actions they are permitted to perform.
pub fn action_vocab_for(actor: &str) -> String {
    let is_pm = matches!(actor, "pm" | "director" | "system");
    let mut lines: Vec<String> = Vec::new();
    let mut current_section: Option<&'static str> = None;

    let fmt_fields = |fields: &[(&str, &str)]| -> String {
        if fields.is_empty() {
            "{}".to_string()
        } else {
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, ty)| format!("  \"{name}\":{ty}"))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    };

    for entry in ACTION_VOCAB {
        if entry.pm_only && !is_pm {
            continue;
        }
        if current_section != Some(entry.section) {
            current_section = Some(entry.section);
            lines.push(entry.section.to_string());
        }
        lines.push(format!("- {}: {}", entry.name, fmt_fields(entry.fields)));
    }

    lines.join("\n")
}
