//! Persona / CV rendering (brief §2.2).
//!
//! A pure renderer turning an agent's *derived* state into a friendly identity
//! card — the persona layer sits ON TOP of the underlying agent configuration
//! and current tasks; it is never a separate source of truth. The event log +
//! projection remain authoritative; this is a view.

use crate::projection::{Projection, TaskStatus};
use crate::runtime::directive;
use serde::Serialize;

/// A friendly identity card for a hired agent, derived from their current work.
#[derive(Debug, Clone, Serialize)]
pub struct Persona {
    pub id: String,
    pub role: String,
    /// "active" while hired (not fired).
    pub status: String,
    /// Role + a short descriptor.
    pub title: String,
    /// Open (non-done) tasks currently assigned.
    pub current_tasks: Vec<String>,
    /// Count of completed (Done) tasks.
    pub completed_tasks: usize,
    /// Titles of the most recent Done tasks (highlights).
    pub highlights: Vec<String>,
    /// Governance directives that apply to this agent's scope.
    pub directives_applicable: Vec<String>,
}

impl Projection {
    /// Render the persona for `agent_id`. Returns None if the agent isn't hired.
    pub fn persona_for(&self, agent_id: &str) -> Option<Persona> {
        let agent = self.agents.iter().find(|a| a.id == agent_id)?;

        let current_tasks: Vec<String> = self
            .tasks
            .iter()
            .filter(|t| t.assignee.as_deref() == Some(agent_id) && t.status != TaskStatus::Done)
            .map(|t| t.id.clone())
            .collect();

        let done: Vec<&crate::projection::Task> = self
            .tasks
            .iter()
            .filter(|t| t.assignee.as_deref() == Some(agent_id) && t.status == TaskStatus::Done)
            .collect();
        let completed_tasks = done.len();
        // Highlights = recently completed work that PASSED review (verified,
        // not just marked done). Unreviewed work still counts toward the tally.
        let highlights: Vec<String> = done
            .iter()
            .rev()
            .filter(|t| t.review.as_ref().map(|r| r.approved).unwrap_or(false))
            .take(3)
            .map(|t| t.title.clone())
            .collect();

        let scopes = self.scopes_for(agent_id);
        let directives_applicable = directive::relevant(self, &scopes)
            .into_iter()
            .map(|d| format!("[{}] {}", d.kind.label(), d.statement))
            .collect();

        Some(Persona {
            id: agent.id.clone(),
            role: agent.role.clone(),
            status: "active".to_string(),
            title: format!(
                "{} — {}",
                agent.role,
                specialize(&agent.role, &current_tasks)
            ),
            current_tasks,
            completed_tasks,
            highlights,
            directives_applicable,
        })
    }
}

/// A short specialization descriptor for the persona title (pure, heuristic).
fn specialize(role: &str, current_tasks: &[String]) -> &'static str {
    if !current_tasks.is_empty() {
        "on active workstreams"
    } else if role.to_lowercase().contains("engineer") {
        "consultant"
    } else if role.to_lowercase().contains("qa") {
        "quality assurance"
    } else {
        "consultant"
    }
}
