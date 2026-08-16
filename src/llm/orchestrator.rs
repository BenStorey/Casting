//! The real LLM orchestrator: implements [`crate::runtime::orchestrator::Orchestrator`]
//! by calling an OpenAI-compatible chat/completions endpoint and parsing the
//! model's reply back into validated `PmAction`s.

use crate::actions::PmAction;
use crate::event::Event;
use crate::llm::client::{ChatMessage, ChatRequest, OpenAiClient};
use crate::llm::config::ProviderConfig;
use crate::runtime::context::AgentContext;
use crate::runtime::orchestrator::{CostMetering, Orchestrator, PlanOutput};
use anyhow::Result;

/// Classify a cost entry by actor role. Uses the agent's CastRole-derived
/// title to classify (C8 fix): pm=overhead, advisor=research, lead=implementation,
/// testing=testing, architect=architecture, stage=tooling, critic=review.
fn classify_cost(actor: &str, agents: &[crate::runtime::context::AgentSummary]) -> String {
    // Look up the actor's role in the agent roster. Fall back to the old
    // heuristic if not found (e.g. system/owner actions).
    let role = agents
        .iter()
        .find(|a| a.id == actor)
        .map(|a| a.role.as_str())
        .unwrap_or("");
    match role {
        "Project Manager" => "pm_overhead",
        "Advisor" => "research",
        "Lead Developer" => "implementation",
        "Testing Engineer" => "testing",
        "Systems Architect" => "architecture",
        "Stage Manager" => "tooling",
        "Critic" => "review",
        // Fallback: old heuristic for owner/system/unknown actors.
        "pm" | "owner" | "system" => "pm_overhead",
        _ => "implementation",
    }
    .into()
}

/// The real provider orchestrator.
///
/// One `Orchestrator` wrapping one provider endpoint. Build a system prompt
/// from a consultant's persona + a Casting planning instruction block, send the
/// assembled `AgentContext`, and parse the returned JSON `{"actions": [...]}`
/// back into `PmAction`s. Every action still flows through `actions::validate`
/// in `pm::run_planned`, so the LLM can only do what it's authorized to.
pub struct LlmOrchestrator {
    http: reqwest::Client,
    resolver: crate::llm::routing::ModelResolver,
    /// Input/output price per 1M tokens, for metering (if known).
    input_price_per_mtok: Option<f64>,
    output_price_per_mtok: Option<f64>,
}

impl LlmOrchestrator {
    pub fn new(client: reqwest::Client, config: ProviderConfig, system_prompt: String) -> Self {
        let resolver = crate::llm::routing::ModelResolver::new(config, Default::default())
            .with_default_persona(system_prompt);
        LlmOrchestrator {
            http: client,
            resolver,
            input_price_per_mtok: None,
            output_price_per_mtok: None,
        }
    }

    /// Route per-actor through a resolver (per-consultant model bindings).
    pub fn with_resolver(mut self, resolver: crate::llm::routing::ModelResolver) -> Self {
        self.resolver = resolver;
        self
    }

    /// The base config (for boot banners / tests).
    pub fn base_config(&self) -> &ProviderConfig {
        self.resolver.base()
    }

    /// Attach metering prices (per 1M input/output tokens) so `estimated_usd`
    /// is real. Optional; without them, cost records a non-zero token count
    /// but `estimated_usd = 0` until a price map is wired.
    pub fn with_prices(mut self, input: f64, output: f64) -> Self {
        self.input_price_per_mtok = Some(input);
        self.output_price_per_mtok = Some(output);
        self
    }

    /// The full action vocabulary for PM/owner actors.
    /// Includes organizational actions (hire, create requirements, provision
    /// worktrees, governance) that only the PM/owner can perform.
    fn full_action_vocab() -> String {
        crate::actions::action_vocab_for("pm")
    }

    /// The subset of actions visible to assignable consultants.
    /// Consultants can work on their assigned tasks, communicate, and record
    /// knowledge — but cannot perform organisational or governance actions.
    fn consultant_action_vocab() -> String {
        crate::actions::action_vocab_for("consultant")
    }

    pub fn planning_instructions(&self, actor: &str) -> String {
        let actions = if matches!(actor, "pm" | "owner" | "system") {
            Self::full_action_vocab()
        } else {
            Self::consultant_action_vocab()
        };

        format!(
            "You are a Casting agent in an autonomous software company.\n\
            \n\
            You respond by emitting a list of VALID actions. Respond ONLY with a JSON \
            object of the form {{\"actions\": [...]}} where each element is ONE of the \
            following, serialized exactly as shown (all fields required unless marked \
            null|optional):\n\
            {actions}\n\
            \n\
            Each action is a typed command the platform validates against policy \
            afterwards. Emit actions that make progress toward the objective given \
            the current operating context. Rules:\n\
            - Include EVERY required field from the shape above; a missing field \
              (e.g. send_message without \"to\") is a hard error.\n\
            - Only emit actions that are legal in the current state (do not complete \
              an unstarted task, do not use an id that already exists).\n\
            - Prefer a small, decisive set of actions.\n\
            - If there is genuinely nothing to do, emit {{\"actions\": []}}.\n\
            - Never invent actions outside the list above.\n\
            - ANTI-THRASH: the operating context lists the decisions already open \
              (open_decisions). Do NOT re-propose a decision whose subject is already \
              open — it would be rejected. Instead, leave it, or supersede a STALE \
              decision you are genuinely replacing via supersede_decision.\n\
            \n\
            Your identity, role, and task-specific workflow are defined in your \
            persona above. Follow your persona's instructions for what actions \
            to take and how to communicate with the PM/owner.\n\
            \n\
            IMPORTANT: output ONLY the JSON object, no prose, no markdown fences."
        )
    }

    /// Parse the model's reply content (the text JSON) into `PmAction`s.
    /// Public so tests can drive parse robustness directly (bare array, fences,
    /// envelope, malformed) without a live server.
    pub fn parse_actions(&self, content: &str) -> Result<Vec<PmAction>> {
        // Defensive: strip any stray markdown fences / surrounding prose.
        let trimmed = content.trim();
        let content = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .map(|s| s.trim().trim_end_matches("```").trim())
            .unwrap_or(trimmed);

        // Accept either a bare array or the {"actions": [...]} envelope.
        let value: serde_json::Value = serde_json::from_str(content)?;
        let arr = match value {
            serde_json::Value::Array(a) => a,
            serde_json::Value::Object(o) => o
                .get("actions")
                .and_then(|v| v.as_array())
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("response missing \"actions\" array"))?,
            _ => anyhow::bail!("unexpected JSON shape from model"),
        };
        arr.iter()
            .map(|v| serde_json::from_value::<PmAction>(v.clone()).map_err(Into::into))
            .collect()
    }
}

#[async_trait::async_trait]
impl Orchestrator for LlmOrchestrator {
    async fn plan(&self, context: &AgentContext, cause: &Event) -> Result<PlanOutput> {
        // ── Step execution mode ──────────────────────────────────────
        // When the actor is executing a playbook step, narrow everything:
        // use the step's model tier, the step prompt, and only the step
        // contract + artifact contents instead of the full company context.
        if let Some(ref step) = context.active_step {
            let step_tier: Option<crate::consultants::CostTier> = match step.model_tier.as_str() {
                "budget" => Some(crate::consultants::CostTier::Budget),
                "standard" => Some(crate::consultants::CostTier::Standard),
                "premium" => Some(crate::consultants::CostTier::Premium),
                _ => None,
            };
            let resolved = self.resolver.resolve(&context.actor, step_tier);
            let client = OpenAiClient::new(
                self.http.clone(),
                resolved.config.base_url.clone(),
                resolved.config.api_key.clone(),
            );

            // Step system prompt: for PM-owned playbooks (chat-interface),
            // use the full planning instruction so the model can either do
            // the work OR escalate (create_task, assign_task, apply_playbook).
            // For consultant-owned steps, use the focused step prompt.
            let step_system = if context.actor == "pm" {
                format!(
                    "{}\n\n{}",
                    resolved.system_prompt,
                    Self::full_action_vocab()
                )
            } else {
                format!(
                    "You are executing step \"{step}\" of playbook \"{pb}\".\n\
                     \n{st_prompt}\n\n\
                     Your task: produce the artifact at \"{artifact}\" in your worktree.\n\
                     Only perform actions relevant to this step. Do NOT plan other work.",
                    step = step.step_title,
                    pb = step.playbook_id,
                    st_prompt = step.step_prompt,
                    artifact = step.produces_artifact,
                )
            };

            // Narrow user payload: step contract + read artifacts, not the
            // full AgentContext dump. For PM-owned steps (chat-interface),
            // also include the owner's original request from the cause event.
            let mut payload_parts = vec![format!(
                "Step: {}\nContract: produce \"{}\"",
                step.step_title, step.produces_artifact
            )];
            if context.actor == "pm" {
                let ask = cause
                    .data
                    .get("body")
                    .and_then(|b| b.as_str())
                    .unwrap_or("")
                    .to_string();
                if !ask.is_empty() {
                    payload_parts.push(format!("Owner request: \"{ask}\""));
                }
            }
            if let Some(ref wt) = step.worktree_path {
                payload_parts.push(format!("Worktree path: {wt}"));
            }
            if !step.reads_artifact_paths.is_empty() {
                payload_parts.push(format!(
                    "Input artifacts to read: {}",
                    step.reads_artifact_paths.join(", ")
                ));
            }

            let req = ChatRequest {
                model: resolved.config.model.clone(),
                messages: vec![
                    ChatMessage {
                        role: "system".into(),
                        content: step_system,
                    },
                    ChatMessage {
                        role: "user".into(),
                        content: payload_parts.join("\n"),
                    },
                ],
                temperature: resolved.temperature,
                max_tokens: resolved.max_tokens,
                response_format: Some(serde_json::json!({"type": "json_object"})),
            };

            let started = std::time::Instant::now();
            let completion = client.chat(&req).await?;
            let latency_ms = started.elapsed().as_millis() as u64;
            let actions = self.parse_actions(&completion.content)?;

            let u = &completion.usage;
            let input_price = self
                .input_price_per_mtok
                .unwrap_or(resolved.input_price_per_mtok);
            let output_price = self
                .output_price_per_mtok
                .unwrap_or(resolved.output_price_per_mtok);

            let metering = CostMetering {
                agent_id: context.actor.clone(),
                task_id: context.my_tasks.first().cloned(),
                cost_class: "playbook".into(),
                model_tier: match resolved.cost_tier {
                    crate::consultants::CostTier::Premium => "premium",
                    crate::consultants::CostTier::Standard => "standard",
                    crate::consultants::CostTier::Budget => "budget",
                }
                .into(),
                model: Some(resolved.config.model.clone()),
                provider: Some(resolved.config.provider.clone()),
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                cache_read_input_tokens: u
                    .prompt_tokens_details
                    .as_ref()
                    .map(|d| d.cached_tokens)
                    .unwrap_or(0),
                cache_creation_input_tokens: u
                    .prompt_tokens_details
                    .as_ref()
                    .map(|d| d.cache_creation_tokens)
                    .unwrap_or(0),
                latency_ms,
                input_price_per_mtok: Some(input_price),
                output_price_per_mtok: Some(output_price),
                estimated_usd: (u.prompt_tokens as f64 * input_price
                    + u.completion_tokens as f64 * output_price)
                    / 1_000_000.0,
            };

            return Ok(PlanOutput {
                actions: actions
                    .into_iter()
                    .map(|a| (context.actor.clone(), a))
                    .collect(),
                metering: Some(metering),
            });
        }

        // ── Normal (non-step) planning mode ──────────────────────────
        // Per-actor routing: the actor decides the model + persona.
        let resolved = self.resolver.resolve(&context.actor, None);
        let client = OpenAiClient::new(
            self.http.clone(),
            resolved.config.base_url.clone(),
            resolved.config.api_key.clone(),
        );

        // The owner's message body (the trigger) must reach the model — the
        // abstracted AgentContext keeps only derived state, whose objective is
        // None before a Requirement exists. The raw ask is the thing to act on.
        let ask = cause
            .data
            .get("body")
            .and_then(|b| b.as_str())
            .unwrap_or("")
            .to_string();
        let user_payload = serde_json::to_string_pretty(context)?;
        let req = ChatRequest {
            model: resolved.config.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: format!(
                        "{}\n\n{}",
                        resolved.system_prompt,
                        self.planning_instructions(&context.actor)
                    ),
                },
                ChatMessage {
                    role: "user".into(),
                    content: format!(
                        "The owner just said: \"{ask}\"\n\n\
                         Current operating context:\n{user_payload}"
                    ),
                },
            ],
            temperature: resolved.temperature,
            max_tokens: resolved.max_tokens,
            response_format: Some(serde_json::json!({"type": "json_object"})),
        };

        let started = std::time::Instant::now();
        let completion = client.chat(&req).await?;
        let latency_ms = started.elapsed().as_millis() as u64;

        let actions = self.parse_actions(&completion.content)?;

        let u = &completion.usage;
        // Per-actor prices: an explicit `with_prices` override wins, else the
        // resolved cost_tier prices (so real LLM spend is non-zero and the
        // budget breaker can trip).
        let input_price = self
            .input_price_per_mtok
            .unwrap_or(resolved.input_price_per_mtok);
        let output_price = self
            .output_price_per_mtok
            .unwrap_or(resolved.output_price_per_mtok);
        let estimated_usd = (u.prompt_tokens as f64 * input_price
            + u.completion_tokens as f64 * output_price)
            / 1_000_000.0;

        let metering = CostMetering {
            agent_id: context.actor.clone(),
            task_id: context.my_tasks.first().cloned(),
            cost_class: classify_cost(&context.actor, &context.agents),
            model_tier: match resolved.cost_tier {
                crate::consultants::CostTier::Premium => "premium",
                crate::consultants::CostTier::Standard => "standard",
                crate::consultants::CostTier::Budget => "budget",
            }
            .into(),
            model: Some(resolved.config.model.clone()),
            provider: Some(resolved.config.provider.clone()),
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            cache_read_input_tokens: u
                .prompt_tokens_details
                .as_ref()
                .map(|d| d.cached_tokens)
                .unwrap_or(0),
            cache_creation_input_tokens: u
                .prompt_tokens_details
                .as_ref()
                .map(|d| d.cache_creation_tokens)
                .unwrap_or(0),
            latency_ms,
            input_price_per_mtok: Some(input_price),
            output_price_per_mtok: Some(output_price),
            estimated_usd,
        };

        Ok(PlanOutput {
            actions: actions
                .into_iter()
                .map(|a| (context.actor.clone(), a))
                .collect(),
            metering: Some(metering),
        })
    }
}
