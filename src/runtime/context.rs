//! Context Assembler (docs/SEMANTIC_EVENTS.md §21).
//!
//! The payoff of the state-core: combine the projection + plan + governance +
//! risks + decisions + tasks into a **targeted operating context per agent or
//! role**, instead of handing an agent the whole event log. Pure derivation —
//! no LLM — but this is exactly the seam the future orchestrator (D2) will read
//! from. Derived, never authoritative: the event log stays the source of truth.

use crate::pm::plan::{PlannedItem, Priority};
use crate::runtime::directive;
use serde::{Deserialize, Serialize};

/// A context item with a computed relevance score (context-assembly scoring).
/// Relevance is a deterministic heuristic — how much this item matters to the
/// receiving actor *right now*: own-task and urgent items score higher. Never
/// authoritative; it's a ranking to shape what the PM/agent pays attention to.
#[derive(Debug, Clone, Serialize)]
pub struct ScoredItem {
    pub task_id: String,
    pub title: String,
    pub priority: String,
    pub status: String,
    pub is_mine: bool,
    /// Composite relevance score (higher = more relevant to this actor now).
    pub relevance: f64,
}

/// A targeted operating context for a single actor (an agent, the PM, or the
/// owner). Surfaces what is *relevant to them*, filtered by governance scope.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentContext {
    pub actor: String,
    pub objective: Option<String>,
    pub priorities: Vec<PlannedItem>,
    /// Priorities annotated with a relevance score for THIS actor (own-task +
    /// urgent items rank highest), so the reader can pay attention in order.
    pub scored_priorities: Vec<ScoredItem>,
    /// Open (non-done) tasks assigned to this actor.
    pub my_tasks: Vec<String>,
    /// Active governance directives that apply to this actor's scope
    /// (via `directive::relevant`, docs/INTENT.md).
    pub active_directives: Vec<String>,
    pub open_risks: Vec<String>,
    pub assumptions: Vec<String>,
    pub constraints: Vec<String>,
    /// Decisions awaiting the owner (the whole open-decision set).
    pub open_decisions: Vec<String>,
    /// EXTERNAL advisor briefings (advisory, never authoritative) scoped to this
    /// actor — e.g. an advisor handoff or a pasted plan. Formatted as
    /// "[advisory · <source>] <title>: <body>". Included so the PM/LLM can
    /// genuinely CONSIDER advice without it ever asserting rules.
    pub advisory_briefings: Vec<String>,
    /// External intake requests (e.g. GitHub issues/PRs) scoped to this actor —
    /// "what a user reported", triaged but NOT the owner's intent.
    pub external_requests: Vec<String>,
    /// The consultant's isolated workspace, if one is provisioned for their
    /// current task (2026-08-12). This is the "desk" the platform hands a
    /// summoned consultant — path, own branch, private build target, and API
    /// port. The agent works HERE, never in the shared checkout.
    pub worktree: Option<WorktreeInfo>,
}

/// The isolated workspace a consultant is handed (subset of `Worktree`,
/// shaped for context consumption — the agent's own desk).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// The task this workspace serves.
    pub task_id: String,
    /// The worktree's own branch.
    pub branch: String,
    /// Path to the worktree (its root).
    pub path: String,
    /// Private CARGO_TARGET_DIR.
    pub cargo_target_dir: String,
    /// The consultant's API port (so their dev server can run in parallel).
    pub port: u16,
}

/// A compact one-line summary of an assembled context — used by the diagnostic
/// `OrchestrationRun` record so a reader can see what the model was handed
/// without re-deriving the whole context.
pub fn summary(ctx: &AgentContext) -> String {
    let obj = ctx
        .objective
        .as_deref()
        .map(|o| {
            if o.len() > 80 {
                format!("{}…", &o[..80])
            } else {
                o.to_string()
            }
        })
        .unwrap_or_else(|| "<no objective>".to_string());
    format!(
        "objective=\"{obj}\"; priorities={} my_tasks={} directives={} risks={} decisions={} briefings={}",
        ctx.priorities.len(),
        ctx.my_tasks.len(),
        ctx.active_directives.len(),
        ctx.open_risks.len(),
        ctx.open_decisions.len(),
        ctx.advisory_briefings.len(),
    )
}

impl crate::projection::Projection {
    /// Assemble the operating context for `actor` (agent id, "owner", or "pm").
    pub fn context_for(&self, actor: &str) -> AgentContext {
        let plan = self.plan();

        // The governance scope this actor operates under. Derive from their
        // task kinds, plus a role default. Heuristic but deterministic.
        let scopes = self.scopes_for(actor);

        let my_tasks = self
            .tasks
            .iter()
            .filter(|t| {
                t.assignee.as_deref() == Some(actor)
                    && t.status != crate::projection::TaskStatus::Done
            })
            .map(|t| t.id.clone())
            .collect::<Vec<_>>();

        // Context-assembly scoring: annotate each priority with how relevant it
        // is to THIS actor right now. Deterministic heuristic: own-task + urgent
        // (blocked / in-review / high-priority) items rank highest.
        let scored_priorities = plan
            .priorities
            .iter()
            .map(|p| ScoredItem {
                task_id: p.task_id.clone(),
                title: p.title.clone(),
                priority: format!("{:?}", p.priority).to_lowercase(),
                status: self
                    .tasks
                    .iter()
                    .find(|t| t.id == p.task_id)
                    .map(|t| format!("{:?}", t.status).to_lowercase())
                    .unwrap_or_default(),
                is_mine: my_tasks.contains(&p.task_id),
                relevance: self.score_for(actor, p),
            })
            .collect::<Vec<_>>();

        let active_directives = directive::relevant(self, &scopes)
            .into_iter()
            .map(|d| format!("[{}] {}", d.kind.label(), d.statement))
            .collect();

        // External advisory briefings (scoped). Format explicitly marks them
        // advisory + provenance so the reader never conflates advice with rules.
        let advisory_briefings = self
            .briefings
            .iter()
            .filter(|b| b.status == crate::types::BriefingStatus::Active)
            .map(|b| format!("[advisory · {}] {}: {}", b.source, b.title, b.body))
            .collect::<Vec<_>>();

        // External intake requests, scoped to this actor's scope areas.
        let external_requests = self
            .external_requests
            .iter()
            .filter(|_| actor == "pm" || actor == "owner" || actor == "system")
            .map(|r| {
                format!(
                    "[external · {}] {} ({}): {}",
                    r.source,
                    r.title,
                    format!("{:?}", r.status).to_lowercase(),
                    r.body
                )
            })
            .collect::<Vec<_>>();

        // The consultant's desk: their most recent non-done task's worktree.
        // The platform provisions it at summon, so the agent is handed a ready
        // isolated workspace (path + branch + build target + port) to work in.
        let worktree = my_tasks
            .last()
            .and_then(|task_id| {
                self.worktrees
                    .iter()
                    .find(|w| w.task_id.as_deref() == Some(task_id))
            })
            .map(|w| WorktreeInfo {
                task_id: w.task_id.clone().unwrap_or_default(),
                branch: w.branch.clone(),
                path: w.path.clone(),
                cargo_target_dir: w.cargo_target_dir.clone(),
                port: w.port,
            });

        AgentContext {
            actor: actor.to_string(),
            objective: plan.objective.clone(),
            priorities: plan.priorities.clone(),
            scored_priorities,
            my_tasks,
            active_directives,
            open_risks: plan.open_risks.clone(),
            assumptions: self.assumptions.iter().map(|a| a.body.clone()).collect(),
            constraints: self.constraints.iter().map(|c| c.body.clone()).collect(),
            open_decisions: plan.open_decisions.clone(),
            advisory_briefings,
            external_requests,
            worktree,
        }
    }

    /// Deterministic relevance score (0..~5) of one planned item to `actor`.
    /// Own-task and urgent items rank highest; owner/PM see everything as relevant.
    fn score_for(&self, actor: &str, p: &PlannedItem) -> f64 {
        let is_mine = self
            .tasks
            .iter()
            .any(|t| t.id == p.task_id && t.assignee.as_deref() == Some(actor));
        let status = self
            .tasks
            .iter()
            .find(|t| t.id == p.task_id)
            .map(|t| t.status);

        // Baseline from priority tier.
        let pbase: f64 = match p.priority {
            Priority::Critical => 3.0,
            Priority::High => 2.0,
            Priority::Medium => 1.0,
            Priority::Low => 0.5,
        };
        // Urgency bump for non-done, active-status items.
        let urgency: f64 = match status {
            Some(crate::projection::TaskStatus::Blocked) => 1.5,
            Some(crate::projection::TaskStatus::InReview) => 1.0,
            Some(crate::projection::TaskStatus::Working) => 0.5,
            _ => 0.0,
        };
        let mine: f64 = if is_mine { 1.0 } else { 0.0 };
        // Owner/PM genuinely care about everything; non-owners get a mild boost
        // only for their own scope.
        let role: f64 = if actor == "owner" || actor == "pm" || actor == "system" {
            0.0
        } else {
            -0.5
        };
        (pbase + urgency + mine + role).max(0.0)
    }

    /// The governance areas an actor operates in: their task kinds (as scope
    /// tokens) plus the scope of their catalog role. Owner/PM see everything.
    pub fn scopes_for(&self, actor: &str) -> Vec<&str> {
        if actor == "owner" || actor == "pm" || actor == "system" {
            return vec!["engineering", "qa", "architecture", "finance", "product"];
        }
        let mut scopes: Vec<&str> = self
            .tasks
            .iter()
            .filter(|t| t.assignee.as_deref() == Some(actor))
            .map(|t| t.kind.as_str())
            .collect();
        // Role default comes from the catalog (the agent's role title maps to a
        // real scope), falling back to a broad-but-safe area.
        if scopes.is_empty() {
            let role_scope = self
                .agents
                .iter()
                .find(|a| a.id == actor)
                .and_then(|a| crate::workspace::role_by_title(&a.role))
                .map(|r| r.scope);
            scopes.push(role_scope.unwrap_or("engineering"));
        }
        scopes
    }
}
