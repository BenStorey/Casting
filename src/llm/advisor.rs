//! Direction Advisor LLM wiring (docs/plans/2026-08-14_d2-routing-advisor-antithrash.md).
//!
//! The advisor's output is a FREE-FORM text reply (not a `PmAction`), so it does
//! NOT go through the `Orchestrator`/gate path. It reuses the SAME
//! [`ModelResolver`] + [`crate::llm::client::OpenAiClient`] so the advisor gets
//! its own model binding (low-volume, top-tier — the economics the role was
//! designed for). The reply lands back in the ISOLATED `advisor_thread`, never
//! in the PM's context until an explicit handoff.

use crate::context::AgentContext;
use crate::llm::client::{ChatMessage, ChatRequest};
use crate::llm::routing::ModelResolver;
use crate::projection::Message;
use anyhow::Result;

/// The outcome of an advisor reply: the text + optional metering.
#[derive(Debug, Clone)]
pub struct AdvisorOutcome {
    pub reply: String,
    pub metering: Option<crate::orchestrator::CostMetering>,
}

/// Generate the advisor's reply to `owner_msg`, given the private thread so far
/// (the advisor's memory) and the resolver (for the advisor's model binding).
///
/// `owner_msg` is the newest owner→advisor message; `thread` is the full
/// isolated thread (including that message) for context. Returns an `Err` on
/// any provider/parse failure — the caller audits it, never panics.
pub async fn advisor_reply(
    resolver: &ModelResolver,
    context: &AgentContext,
    thread: &[Message],
    owner_msg: &str,
) -> Result<AdvisorOutcome> {
    let resolved = resolver.resolve("advisor");

    // The advisor's memory = the private thread. Include it verbatim so it can
    // continue the strategic conversation.
    let mut messages = vec![ChatMessage {
        role: "system".into(),
        content: resolved.system_prompt,
    }];
    for m in thread.iter().rev().take(20).rev() {
        let role = if m.from == "owner" {
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

    let client = crate::llm::client::OpenAiClient::new(
        resolved.config.base_url.clone(),
        resolved.config.api_key.clone(),
    );
    let req = ChatRequest {
        model: resolved.config.model.clone(),
        messages,
        temperature: None,
        max_tokens: None,
        response_format: None,
    };

    let started = std::time::Instant::now();
    let completion = client.chat(&req).await?;
    let latency_ms = started.elapsed().as_millis() as u64;
    let reply = completion.content.trim().to_string();
    if reply.is_empty() {
        anyhow::bail!("advisor returned an empty reply");
    }

    let u = &completion.usage;
    let metering = Some(crate::orchestrator::CostMetering {
        agent_id: "advisor".into(),
        task_id: None,
        model_tier: "premium".into(),
        model: Some(resolved.config.model.clone()),
        provider: Some(resolved.config.provider.clone()),
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        cache_read_input_tokens: u
            .prompt_tokens_details
            .as_ref()
            .map(|d| d.cached_tokens)
            .unwrap_or(0),
        cache_creation_input_tokens: 0,
        latency_ms,
        input_price_per_mtok: None,
        output_price_per_mtok: None,
        estimated_usd: 0.0,
    });

    let _ = context; // reserved: future advisor context assembly (high-level state)
    Ok(AdvisorOutcome { reply, metering })
}
