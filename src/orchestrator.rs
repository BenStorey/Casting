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
use anyhow::Result;

/// Provider metering for one orchestrator call (HARNESS #6 — cost attribution
/// & token budgeting). Returned alongside actions so the PM can land it in the
/// event log as a `CostIncurred` event; spend becomes attributable per
/// agent/task and feeds the PM's budget concern.
#[derive(Debug, Clone, PartialEq)]
pub struct CostMetering {
    /// The agent whose call incurred this spend (e.g. "pm", "marcus-reed").
    pub agent_id: String,
    /// The task this call is attributed to, if any.
    pub task_id: Option<String>,
    /// Model tier, e.g. "flash" | "pro" (from the provider).
    pub model_tier: String,
    /// Exact model id that ran (e.g. "deepseek/deepseek-v4-flash-0731").
    pub model: Option<String>,
    /// Provider that served the call (e.g. "openrouter").
    pub provider: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Input tokens READ FROM the provider's prompt cache (a cache "hit" — cheap).
    pub cache_read_input_tokens: u64,
    /// Input tokens WRITTEN to the provider's prompt cache (a "creation" — not a hit).
    pub cache_creation_input_tokens: u64,
    /// Wall-clock duration of the call in milliseconds (0 = unknown).
    pub latency_ms: u64,
    /// Per-1M-token rates used to compute `estimated_usd` (for reconstruction).
    pub input_price_per_mtok: Option<f64>,
    pub output_price_per_mtok: Option<f64>,
    /// Estimated USD cost of the call.
    pub estimated_usd: f64,
}

/// The result of an orchestrator call: the planned actions PLUS the metering
/// for the call that produced them (when a provider was actually used). When
/// no provider call happened (deterministic/scripted planning, e.g. the mock
/// being stateless), `metering` is `None` and no cost is recorded.
#[derive(Debug, Clone, Default)]
pub struct PlanOutput {
    pub actions: Vec<PlannedAction>,
    pub metering: Option<CostMetering>,
}

/// The D2 contract: turn an assembled operating context + the triggering event
/// into planned actions. The output is still validated by the policy gate, so
/// an LLM (or anything) can only do what it's authorized to.
///
/// Async + fallible: a real provider call is a network round-trip that can fail
/// (timeout, rate limit, malformed reply). An `Err` means the pass produced no
/// actions and (for a real call) the metering/audit records the failure — the
/// caller decides how to surface it.
#[async_trait::async_trait]
pub trait Orchestrator: Send + Sync {
    /// Plan the PM's response to `cause`, given the assembled context for the
    /// actor being orchestrated.
    async fn plan(&self, context: &AgentContext, cause: &Event) -> Result<PlanOutput>;
}

/// A deterministic stand-in for the LLM. Drives a minimal, scripted PM loop:
/// acknowledge owner messages and (after the first build) propose a follow-up
/// decision. Turns the seam on end-to-end with no live model. Stateless (the
/// mock derives everything deterministically from the context).
#[derive(Debug, Clone, Copy, Default)]
pub struct MockOrchestrator;

#[async_trait::async_trait]
impl Orchestrator for MockOrchestrator {
    async fn plan(&self, context: &AgentContext, cause: &Event) -> Result<PlanOutput> {
        let mut actions: Vec<PlannedAction> = Vec::new();

        if context.objective.is_none() {
            // No product objective yet: acknowledge and wait for direction.
            let body = cause
                .data
                .get("body")
                .and_then(|b| b.as_str())
                .unwrap_or("");
            actions.push((
                "pm".into(),
                PmAction::SendMessage {
                    to: "owner".into(),
                    body: format!("On it — \u{201c}{body}\u{201d}. I'll scope it into tasks."),
                },
            ));
            return Ok(PlanOutput {
                actions,
                // The mock is deterministic/stateless — no real provider call, so
                // no cost to record (the seam is exercised but spend stays zero).
                metering: None,
            });
        }

        // There's an objective: narrow the task backlog (mock "reasoning").
        if context.priorities.is_empty() {
            actions.push((
                "pm".into(),
                PmAction::CreateTask {
                    id: "task-mock-1".into(),
                    title: "Implement the build plan".to_string(),
                    kind: "feature".into(),
                },
            ));
        } else {
            actions.push((
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

        Ok(PlanOutput {
            actions,
            metering: Some(CostMetering {
                agent_id: "pm".into(),
                task_id: None,
                model_tier: "flash".into(),
                model: Some("deepseek/deepseek-v4-flash-0731".into()),
                provider: Some("openrouter".into()),
                prompt_tokens: 1200,
                completion_tokens: 300,
                cache_read_input_tokens: 200,
                cache_creation_input_tokens: 100,
                latency_ms: 150,
                input_price_per_mtok: Some(0.25),
                output_price_per_mtok: Some(1.25),
                estimated_usd: 0.0018,
            }),
        })
    }
}
