//! Context Assembler (docs/SEMANTIC_EVENTS.md §21).
//!
//! The payoff of the state-core: combine the projection + plan + governance +
//! risks + decisions + tasks into a **targeted operating context per agent or
//! role**, instead of handing an agent the whole event log. Pure derivation —
//! no LLM — but this is exactly the seam the future orchestrator (D2) will read
//! from. Derived, never authoritative: the event log stays the source of truth.

use crate::directive;
use crate::plan::PlannedItem;
use serde::Serialize;

/// A targeted operating context for a single actor (an agent, the PM, or the
/// owner). Surfaces what is *relevant to them*, filtered by governance scope.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentContext {
    pub actor: String,
    pub objective: Option<String>,
    pub priorities: Vec<PlannedItem>,
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

        let active_directives = directive::relevant(self, &scopes)
            .into_iter()
            .map(|d| format!("[{}] {}", d.kind.label(), d.statement))
            .collect();

        AgentContext {
            actor: actor.to_string(),
            objective: plan.objective.clone(),
            priorities: plan.priorities.clone(),
            my_tasks,
            active_directives,
            open_risks: plan.open_risks.clone(),
            assumptions: self.assumptions.iter().map(|a| a.body.clone()).collect(),
            constraints: self.constraints.iter().map(|c| c.body.clone()).collect(),
            open_decisions: plan.open_decisions.clone(),
        }
    }

    /// The governance areas an actor operates in: their task kinds (as scope
    /// tokens) plus a per-role default. Owner/PM see everything.
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
        // Role default falls back to a broad-but-safe area.
        if scopes.is_empty() {
            scopes.push(role_default(actor));
        }
        scopes
    }
}

fn role_default(actor: &str) -> &'static str {
    match actor {
        a if a.starts_with("marcus") || a.contains("engineer") => "engineering",
        a if a.starts_with("maya") || a.contains("qa") => "qa",
        _ => "engineering",
    }
}
