use super::append_json;
use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

/// POST /api/advisor/message input: an owner→advisor message. Appends to the
/// private advisor thread, ISOLATED from the PM's context until a handoff.
#[derive(Deserialize)]
pub(crate) struct AdvisorMsgIn {
    body: String,
}

/// POST /api/advisor/message — an owner→advisor message. Appends to the PRIVATE
/// advisor thread, which is isolated from the PM's context until a handoff.
///
/// When the LLM is configured, also generates the advisor's reply (via the
/// advisor model binding + the private thread) and appends it as an
/// `AdvisorMessageSent` from the advisor — so the owner↔advisor conversation is
/// real. A blocked/failed call is audited and produces no reply (no panic).
pub(crate) async fn advisor_message_handler(
    State(state): State<AppState>,
    Json(input): Json<AdvisorMsgIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let body = input.body.trim().to_string();
    if body.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "advisor message must not be empty".into(),
        ));
    }
    let owner_ev = Event::new(
        &state.project,
        Actor::Owner,
        EventType::AdvisorMessageSent,
        Aggregate {
            kind: "advisor_thread".into(),
            id: format!("am-{}", uuid::Uuid::new_v4()),
        },
        serde_json::json!({ "to": "advisor", "body": body }),
    );
    let stored = state
        .append(owner_ev)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Attempt a real advisor reply if the LLM is configured.
    maybe_advisor_reply(&state, &body).await;

    Ok(Json(stored))
}

/// If the LLM is configured, generate the advisor's reply to `owner_body` and
/// append it + its cost to the thread. Best-effort: failures/guard blocks are
/// audited silently (a reply is a nice-to-have, never load-bearing).
async fn maybe_advisor_reply(state: &AppState, owner_body: &str) {
    let Some(base_cfg) = crate::llm::config::from_env(state.state_dir.as_deref()).ok().flatten() else {
        return; // LLM not configured — deterministic (no reply).
    };
    // Harness guard: paused / budget-halted → no provider call, no spend.
    let proj = match state.projection() {
        Ok(p) => p,
        Err(_) => return,
    };
    if let Err(reason) = crate::pm::guard::llm_dispatch_allowed(&proj) {
        eprintln!("[advisor] guard blocked LLM dispatch: {reason}");
        return;
    }
    let resolver = crate::llm::routing::ModelResolver::new(base_cfg, (*state.consultants).clone());
    let thread = proj.advisor_thread.clone();
    // Ground the advisor in the real high-level operating context (objective,
    // governance, risks, decisions) — NOT task machinery (the advisor's role
    // operates above task priorities).
    let advisor_context = proj.context_for("advisor");
    let outcome = crate::llm::advisor_reply(&resolver, &advisor_context, &thread, owner_body).await;
    match outcome {
        Ok(outcome) => {
            let reply_ev = Event::new(
                &state.project,
                Actor::Agent {
                    id: "advisor".into(),
                },
                EventType::AdvisorMessageSent,
                Aggregate {
                    kind: "advisor_thread".into(),
                    id: format!("am-{}", uuid::Uuid::new_v4()),
                },
                serde_json::json!({ "to": "owner", "body": outcome.reply }),
            );
            if let Err(e) = state.append(reply_ev) {
                eprintln!("[advisor] failed to append reply: {e:#}");
            }
            // Cost attribution (metering from the advisor call).
            if let Some(m) = outcome.metering {
                let _ = state.append(crate::event::Event::new(
                    &state.project,
                    Actor::System,
                    EventType::CostIncurred,
                    Aggregate {
                        kind: "cost".into(),
                        id: uuid::Uuid::new_v4().to_string(),
                    },
                    serde_json::json!({
                        "agent_id": m.agent_id,
                        "task_id": m.task_id,
                        "model_tier": m.model_tier,
                        "model": m.model,
                        "provider": m.provider,
                        "prompt_tokens": m.prompt_tokens,
                        "completion_tokens": m.completion_tokens,
                        "cache_read_input_tokens": m.cache_read_input_tokens,
                        "cache_creation_input_tokens": m.cache_creation_input_tokens,
                        "latency_ms": m.latency_ms,
                        "input_price_per_mtok": m.input_price_per_mtok,
                        "output_price_per_mtok": m.output_price_per_mtok,
                        "estimated_usd": m.estimated_usd,
                    }),
                ));
            }
        }
        Err(err) => {
            eprintln!("[advisor] LLM reply failed (audited): {err:#}");
            let _ = state.append(Event::new(
                &state.project,
                Actor::System,
                EventType::OrchestrationRun,
                Aggregate {
                    kind: "plan".into(),
                    id: format!("run-{}", uuid::Uuid::new_v4()),
                },
                serde_json::json!({
                    "trigger": "AdvisorMessageSent",
                    "actor": "advisor",
                    "error": format!("{err:#}"),
                    "metered": false,
                }),
            ));
        }
    }
}

/// POST /api/advisor/handoff input: turn the advisor thread into a Briefing
/// the PM reads. `summary` is the (owner/LLM) distilled take; we record it as
/// an AdvisoryBriefing provenanced "advisor".
#[derive(Deserialize)]
pub(crate) struct AdvisorHandoffIn {
    title: Option<String>,
    subject: Option<String>,
    summary: String,
}

/// POST /api/advisor/handoff — turn the owner↔advisor strategic conversation into
/// an AdvisoryBriefing the PM DOES read (source "advisor"). This is the explicit
/// integration point between the owner's two direct roles (PM + advisor).
pub(crate) async fn advisor_handoff_handler(
    State(state): State<AppState>,
    Json(input): Json<AdvisorHandoffIn>,
) -> Result<Json<Event>, (StatusCode, String)> {
    let summary = input.summary.trim().to_string();
    if summary.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "handoff summary must not be empty".into(),
        ));
    }
    let subject = input.subject.unwrap_or_default().trim().to_string();
    let ev = Event::new(
        &state.project,
        Actor::Owner,
        EventType::AdvisorHandoff,
        Aggregate {
            kind: "briefing".into(),
            id: format!("brief-{}", uuid::Uuid::new_v4()),
        },
        serde_json::json!({
            "source": "advisor",
            "subject": subject,
            "title": input.title.unwrap_or_else(|| "Advisor handoff".into()),
            "body": summary,
        }),
    );
    append_json(&state, ev)
}

/// POST /api/advisor/summarize — have the LLM distill the owner↔advisor thread
/// into a concise briefing summary (used to pre-fill the handoff). Falls back
/// to the deterministic summarizer (or "nothing to summarize") when there's no
/// LLM or the call fails — never a hard error.
pub(crate) async fn advisor_summarize_handler(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let thread = proj.advisor_thread.clone();
    let fallback = crate::llm::advisor_summarize_deterministic(&thread);
    let Some(base_cfg) = crate::llm::config::from_env(state.state_dir.as_deref()).ok().flatten() else {
        return Ok(Json(serde_json::json!({ "summary": fallback })));
    };
    let resolver = crate::llm::routing::ModelResolver::new(base_cfg, (*state.consultants).clone());
    match crate::llm::advisor_summarize(&resolver, &thread).await {
        Ok(summary) if !summary.trim().is_empty() => {
            Ok(Json(serde_json::json!({ "summary": summary })))
        }
        // Empty/broken LLM summary → deterministic fallback (never explode).
        _ => Ok(Json(serde_json::json!({ "summary": fallback }))),
    }
}
