//! Direction Advisor LLM wiring (docs/plans/2026-08-14_d2-routing-advisor-antithrash.md).
//!
//! The advisor's output is a FREE-FORM text reply (not a `PmAction`), so it does
//! NOT go through the `Orchestrator`/gate path. It reuses the SAME
//! [`ModelResolver`] + [`crate::llm::client::OpenAiClient`] so the advisor gets
//! its own model binding (low-volume, top-tier — the economics the role was
//! designed for). The reply lands back in the ISOLATED `advisor_thread`, never
//! in the PM's context until an explicit handoff.

use crate::llm::client::{ChatMessage, ChatRequest};
use crate::llm::routing::ModelResolver;
use crate::projection::Message;
use crate::runtime::context::AgentContext;
use anyhow::Result;

/// The outcome of an advisor reply: the text + optional metering.
#[derive(Debug, Clone)]
pub struct AdvisorOutcome {
    pub reply: String,
    pub metering: Option<crate::runtime::orchestrator::CostMetering>,
    /// The fully-assembled prompt sent to the model, for out-of-band archival.
    pub prompt: Option<String>,
    /// The raw model completion text, for out-of-band archival.
    pub response: Option<String>,
}

/// Generate the advisor's reply to `owner_msg`, given the private thread so far
/// (the advisor's memory) and the resolver (for the advisor's model binding).
///
/// `owner_msg` is the newest director→advisor message; `thread` is the full
/// isolated thread (including that message) for context; `context` is the
/// assembled operating context (objective / governance / risks / decisions)
/// the advisor grounds its advice in. Returns an `Err` on any provider/parse
/// failure — the caller audits it, never panics.
pub async fn advisor_reply(
    client: &reqwest::Client,
    resolver: &ModelResolver,
    context: &AgentContext,
    thread: &[Message],
    owner_msg: &str,
) -> Result<AdvisorOutcome> {
    // The advisor's actor id — resolved by role from the context; fall back to
    // the role-resolved constant when the context is minimal (e.g. tests pass a
    // default AgentContext with no advisor agent populated).
    let advisor = if context.advisor_id.is_empty() {
        crate::actions::advisor_actor_id(None).to_string()
    } else {
        context.advisor_id.clone()
    };
    let resolved = resolver.resolve(&advisor, None);

    // The advisor's memory = the private thread. Include it verbatim so it can
    // continue the strategic conversation.
    let mut messages = vec![ChatMessage {
        role: "system".into(),
        content: format!(
            "{}\n\n## Current operating context (for grounding your advice)\n{}",
            resolved.system_prompt,
            advisor_context_summary(context)
        ),
    }];
    for m in thread.iter().rev().take(20).rev() {
        let role = if m.from == "director" {
            "user"
        } else {
            "assistant"
        };
        messages.push(ChatMessage {
            role: role.into(),
            content: m.body.clone(),
        });
    }
    // Make sure the current ask is last and clearly marked as the thing to
    // respond to (in case it's not the tail of the sliced thread).
    let ask_is_last = thread.last().map(|m| m.body == owner_msg).unwrap_or(false);
    if !ask_is_last {
        messages.push(ChatMessage {
            role: "user".into(),
            content: owner_msg.to_string(),
        });
    }

    let client = crate::llm::client::Client::new(
        &resolved.config.provider,
        client.clone(),
        resolved.config.base_url.clone(),
        resolved.config.api_key.clone(),
    );
    let req = ChatRequest {
        model: resolved.config.model.clone(),
        messages,
        temperature: resolved.temperature,
        max_tokens: resolved.max_tokens,
        response_format: None,
    };

    // The exact prompt bytes as sent — surfaced on `AdvisorOutcome` so the
    // caller can archive it out-of-band (prompt_archive).
    let prompt_string = req
        .messages
        .iter()
        .map(|m| format!("[{}]\n{}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n\n");

    let started = std::time::Instant::now();
    let completion = client.chat(&req).await?;
    let latency_ms = started.elapsed().as_millis() as u64;
    let reply = completion.content.trim().to_string();
    if reply.is_empty() {
        anyhow::bail!("advisor returned an empty reply");
    }

    let u = &completion.usage;
    let prices = crate::llm::pricing::CostPrices::from_halves(
        resolved.input_price_per_mtok,
        resolved.output_price_per_mtok,
    );
    let metering = Some(crate::llm::pricing::metering(
        advisor.clone(),
        None,
        "research".into(),
        "premium".into(),
        resolved.config.model.clone(),
        resolved.config.provider.clone(),
        u,
        latency_ms,
        prices,
        completion.usage.cost,
    ));

    Ok(AdvisorOutcome {
        reply,
        metering,
        prompt: Some(prompt_string),
        response: Some(completion.content.clone()),
    })
}

/// Curate the HIGH-LEVEL operating context the advisor grounds its advice in:
/// the objective, active governance, open risks/assumptions/constraints, and
/// open decisions — deliberately NOT task machinery (the advisor operates
/// ABOVE task priorities by design).
pub fn advisor_context_summary(context: &AgentContext) -> String {
    let mut out = Vec::new();
    out.push(format!(
        "- Objective: {}",
        context.objective.as_deref().unwrap_or("<none set>")
    ));
    if context.advisory_briefings.is_empty() {
        out.push("- Prior advisory briefings: none".to_string());
    } else {
        out.push(format!(
            "- Prior advisory briefings:\n    {}",
            context.advisory_briefings.join("\n    ")
        ));
    }
    if context.active_directives.is_empty() {
        out.push("- Governance: none active".to_string());
    } else {
        out.push(format!(
            "- Governance: {}",
            context.active_directives.join("; ")
        ));
    }
    out.push(format!(
        "- Open decisions: {}",
        if context.open_decisions.is_empty() {
            "none".to_string()
        } else {
            context.open_decisions.join("; ")
        }
    ));
    out.push(format!(
        "- Open risks: {}",
        if context.open_risks.is_empty() {
            "none".to_string()
        } else {
            context.open_risks.join("; ")
        }
    ));
    out.push(format!(
        "- Assumptions: {}",
        if context.assumptions.is_empty() {
            "none".to_string()
        } else {
            context.assumptions.join("; ")
        }
    ));
    out.push(format!(
        "- Constraints: {}",
        if context.constraints.is_empty() {
            "none".to_string()
        } else {
            context.constraints.join("; ")
        }
    ));
    out.join("\n")
}

/// Produce a concise, faithful summary of the director↔advisor thread — used for
/// the handoff briefing the PM reads. Reuses the advisor's model binding.
/// Returns an `Err` on any provider/parse failure; the caller falls back to the
/// deterministic summarizer (never a hard failure).
pub async fn advisor_summarize(
    client: &reqwest::Client,
    resolver: &ModelResolver,
    thread: &[Message],
) -> Result<String> {
    let resolved = resolver.resolve(crate::actions::advisor_actor_id(None), None);
    let mut messages = vec![ChatMessage {
        role: "system".into(),
        content: format!(
            "{}\n\nSummarize the director's advisor conversation below into a concise, \
             faithful briefing (3–6 sentences) for a project manager to act on. \
             Preserve any concrete decisions, options weighed, and open questions.\
             ",
            resolved.system_prompt
        ),
    }];
    for m in thread.iter().rev().take(20).rev() {
        let role = if m.from == "director" {
            "user"
        } else {
            "assistant"
        };
        messages.push(ChatMessage {
            role: role.into(),
            content: m.body.clone(),
        });
    }
    if messages.len() == 1 {
        // No actual content — nothing to summarize.
        return Ok("Advisor conversation handed off to PM.".to_string());
    }

    let client = crate::llm::client::Client::new(
        &resolved.config.provider,
        client.clone(),
        resolved.config.base_url.clone(),
        resolved.config.api_key.clone(),
    );
    let req = ChatRequest {
        model: resolved.config.model.clone(),
        messages,
        temperature: resolved.temperature,
        max_tokens: resolved.max_tokens,
        response_format: None,
    };
    let completion = client.chat(&req).await?;
    let summary = completion.content.trim().to_string();
    if summary.is_empty() {
        anyhow::bail!("advisor summarize returned an empty summary");
    }
    Ok(summary)
}

/// The deterministic fallback summarizer (when no LLM or the call fails):
/// headings from the director's messages. Mirrors the (previously frontend-only)
/// logic server-side so the summarize endpoint always has a value to return.
pub fn advisor_summarize_deterministic(thread: &[Message]) -> String {
    if thread.is_empty() {
        return "Advisor conversation handed off to PM.".to_string();
    }
    let owners: Vec<String> = thread
        .iter()
        .filter(|m| m.from == "director")
        .map(|m| m.body.clone())
        .collect();
    if owners.is_empty() {
        "Advisor conversation handed off to PM.".to_string()
    } else {
        format!("Advisor session — owner's thinking: {}", owners.join("; "))
    }
}
