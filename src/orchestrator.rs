//! The orchestrator seam (D2) — the contract the future LLM PM will implement.
//!
//! Every piece so far was shaped for this: the PM's "reasoning" is isolated in
//! a few scripted `plan_*` functions that take (state, cause, policy) and
//! return `PmAction`s. D2 replaces (or augments) those with a real LLM. This
//! module defines the *contract* — [`Orchestrator`] — and ships a deterministic
//! [`MockOrchestrator`] so the whole seam is built, tested, and gate-checked
//! with **zero LLM and zero spend**.
//!
//! The real OpenRouter provider stays UNSET here (the owner connects it later,
//! while travelling the LLM is deliberately unplugged). When it's plugged in,
//! it just has to implement [`Orchestrator`]: read the assembled context, return
//! `PmAction`s, and the existing gate + append path do the rest.

use crate::actions::PmAction;
use crate::context::AgentContext;
use crate::event::Event;
use crate::pm::PlannedAction;

/// The D2 contract: turn an assembled operating context + the triggering event
/// into planned actions. The output is still validated by the policy gate, so
/// an LLM (or anything) can only do what it's authorized to.
pub trait Orchestrator: Send + Sync {
    /// Plan the PM's response to `cause`, given the assembled context for the
    /// actor being orchestrated.
    fn plan(&self, context: &AgentContext, cause: &Event) -> Vec<PlannedAction>;
}

/// A deterministic stand-in for the LLM. Drives a minimal, scripted PM loop:
/// acknowledge owner messages and (after the first build) propose a follow-up
/// decision. Turns the seam on end-to-end with no live model. Stateless (the
/// mock derives everything deterministically from the context).
#[derive(Debug, Clone, Copy, Default)]
pub struct MockOrchestrator;

impl Orchestrator for MockOrchestrator {
    fn plan(&self, context: &AgentContext, cause: &Event) -> Vec<PlannedAction> {
        let mut out: Vec<PlannedAction> = Vec::new();

        if context.objective.is_none() {
            // No product objective yet: acknowledge and wait for direction.
            let body = cause
                .data
                .get("body")
                .and_then(|b| b.as_str())
                .unwrap_or("");
            out.push((
                "pm".into(),
                PmAction::SendMessage {
                    to: "owner".into(),
                    body: format!("On it — \u{201c}{body}\u{201d}. I'll scope it into tasks."),
                },
            ));
            return out;
        }

        // There's an objective: narrow the task backlog (mock "reasoning").
        if context.priorities.is_empty() {
            out.push((
                "pm".into(),
                PmAction::CreateTask {
                    id: "task-mock-1".into(),
                    title: "Implement the build plan".to_string(),
                    kind: "feature".into(),
                },
            ));
        } else {
            out.push((
                "pm".into(),
                PmAction::ProposeDecision {
                    id: "decision-mock-1".into(),
                    subject: "Mock follow-up".to_string(),
                    options: serde_json::json!({ "A": "continue", "B": "pause" }),
                    recommendation: "A".into(),
                    class: crate::policy::DecisionClass::InternalImplementation,
                    involvement: crate::policy::OwnerInvolvement::Pm,
                },
            ));
        }

        out
    }
}
