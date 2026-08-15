use crate::pm::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

/// GET /api/consultants — the loaded consultant registry (embedded defaults +
/// any user overlay from `.casting/consultants/`): identity, role, system
/// prompt, routing hints, model binding, verification. Configuration, never
/// authority (who's hired stays in the event log). Read by the D2 orchestrator
/// and the UI.
pub(crate) async fn consultants_handler(
    State(state): State<AppState>,
) -> Json<Vec<crate::consultants::ConsultantConfig>> {
    Json(state.consultants.all().into_iter().cloned().collect())
}

/// The per-actor EFFECTIVE model routing — what each actor will actually run
/// on after env-fallback resolution (not just the raw package binding). Lets
/// the UI show "Marcus → cheap-model (openrouter)" vs "advisor → premium".
#[derive(serde::Serialize)]
pub(crate) struct ActorRouting {
    pub actor: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Per-1M-token prices used to meter this actor (from cost_tier).
    pub input_price_per_mtok: f64,
    pub output_price_per_mtok: f64,
}

/// GET /api/routing — resolve the per-actor model routing from the env base
/// config + the consultant registry (the same resolver `cast run` builds).
/// Read-only debug surface: which model each actor is handed.
pub(crate) async fn routing_handler(State(state): State<AppState>) -> Json<Vec<ActorRouting>> {
    let Some(base_cfg) = crate::llm::config::from_env().ok().flatten() else {
        return Json(Vec::new()); // LLM not configured — nothing to route.
    };
    let resolver = crate::llm::routing::ModelResolver::new(base_cfg, (*state.consultants).clone());
    // Resolve for every known consultant id, plus pm and advisor.
    let mut actors: Vec<String> = state
        .consultants
        .all()
        .iter()
        .map(|c| c.id.clone())
        .collect();
    actors.push("pm".into());
    actors.push("advisor".into());
    actors.sort();
    actors.dedup();

    let rows = actors
        .into_iter()
        .map(|actor| {
            let r = resolver.resolve(&actor);
            ActorRouting {
                actor,
                provider: r.config.provider,
                model: r.config.model,
                base_url: r.config.base_url,
                temperature: r.temperature,
                max_tokens: r.max_tokens,
                input_price_per_mtok: r.input_price_per_mtok,
                output_price_per_mtok: r.output_price_per_mtok,
            }
        })
        .collect();
    Json(rows)
}

/// GET /api/context/{actor} — the assembled operating context for an actor
/// (agent id, "owner", or "pm"): objective, priorities, their tasks, the
/// governance directives that apply to them, risks, and open decisions.
pub(crate) async fn context_handler(
    State(state): State<AppState>,
    Path(actor): Path<String>,
) -> Result<Json<crate::context::AgentContext>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(proj.context_for(&actor)))
}

/// GET /api/persona/{agent_id} — the derived persona/CV card for a hired agent.
pub(crate) async fn persona_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<crate::persona::Persona>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match proj.persona_for(&agent_id) {
        Some(p) => Ok(Json(p)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// GET /api/graph — the derived graph view: nodes + parallel-work groups
/// (join points) + active/blocked tokens + per-node provenance chains and
/// currently-available transitions. The graph/transition spine (visualization
/// + "what's stuck" + "why in this order"). Pure derivation, no authority.
pub(crate) async fn graph_handler(
    State(state): State<AppState>,
) -> Result<Json<crate::graph::GraphView>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(proj.graph()))
}

/// GET /api/graph/task/{id} — the narrow PM planning context for ONE task:
/// its derived state, a short report, and its currently-valid transitions
/// ("which transition and why?"). The D2 prompt seam.
pub(crate) async fn graph_task_context_handler(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<crate::graph::PmTaskContext>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match proj.pm_task_context(&task_id) {
        Some(ctx) => Ok(Json(ctx)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// GET /api/model — the operating picture: what the models are currently
/// seeing (objective, priorities, governance, knowledge, per-actor contexts,
/// and any mechanical drift signals). The owner's "why is it prioritizing that
/// way?" / "what does it believe?" debug surface. Pure derivation.
pub(crate) async fn model_handler(
    State(state): State<AppState>,
) -> Result<Json<crate::mental::OperatingModel>, StatusCode> {
    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(proj.operating_model()))
}

/// The full assembled context that would be sent to an actor's LLM:
/// system prompt (persona) + planning instructions (action vocabulary) +
/// the structured AgentContext. This is the actual text the model sees,
/// assembled exactly as the orchestrator would build it.
#[derive(serde::Serialize)]
pub(crate) struct FullActorContext {
    pub actor: String,
    pub system_prompt: String,
    pub planning_instructions: String,
    pub agent_context: crate::context::AgentContext,
    pub assembled_context: String,
}

/// GET /api/debug/context/{actor} — the FULL prompt the model would receive.
/// Combines the actor's persona, the action vocabulary, and the structured
/// AgentContext into one readable block so you can eyeball what the model
/// actually sees and assess context bloat.
pub(crate) async fn full_context_handler(
    State(state): State<AppState>,
    Path(actor): Path<String>,
) -> Result<Json<FullActorContext>, StatusCode> {
    use crate::llm::orchestrator::LlmOrchestrator;
    use crate::llm::config::ProviderConfig;

    let proj = state
        .projection()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let agent_ctx = proj.context_for(&actor);

    // Resolve the system prompt / persona for this actor.
    let system_prompt = state
        .consultants
        .by_id(&actor)
        .and_then(|c| c.system_prompt.clone())
        .or_else(|| {
            // PM and advisor have special handling.
            if actor == "pm" || actor == "advisor" {
                state
                    .consultants
                    .by_id("pm")
                    .and_then(|c| c.system_prompt.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();

    // Build the planning instructions (same function the real orchestrator uses).
    let base_cfg = ProviderConfig {
        provider: "null".into(),
        model: "null".into(),
        base_url: String::new(),
        api_key: String::new(),
    };
    let orch = LlmOrchestrator::new(base_cfg, system_prompt.clone());
    let planning = orch.planning_instructions(&actor);

    // Assemble the full prompt.
    let assembled = format!(
        "{}\n\n{}\n\n# Current Operating Context\n{}",
        system_prompt,
        planning,
        serde_json::to_string_pretty(&agent_ctx).unwrap_or_default()
    );

    Ok(Json(FullActorContext {
        actor,
        system_prompt,
        planning_instructions: planning,
        agent_context: agent_ctx,
        assembled_context: assembled,
    }))
}
