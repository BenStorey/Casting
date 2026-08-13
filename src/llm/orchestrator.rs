//! The real LLM orchestrator: implements [`crate::orchestrator::Orchestrator`]
//! by calling an OpenAI-compatible chat/completions endpoint and parsing the
//! model's reply back into validated `PmAction`s.

use crate::actions::PmAction;
use crate::context::AgentContext;
use crate::event::Event;
use crate::llm::client::{ChatMessage, ChatRequest, OpenAiClient};
use crate::llm::config::ProviderConfig;
use crate::orchestrator::{CostMetering, Orchestrator, PlanOutput};
use anyhow::Result;

/// The real provider orchestrator.
///
/// One `Orchestrator` wrapping one provider endpoint. Build a system prompt
/// from a consultant's persona + a Casting planning instruction block, send the
/// assembled `AgentContext`, and parse the returned JSON `{"actions": [...]}`
/// back into `PmAction`s. Every action still flows through `actions::validate`
/// in `pm::run_planned`, so the LLM can only do what it's authorized to.
pub struct LlmOrchestrator {
    client: OpenAiClient,
    config: ProviderConfig,
    system_prompt: String,
    /// Input/output price per 1M tokens, for metering (if known).
    input_price_per_mtok: Option<f64>,
    output_price_per_mtok: Option<f64>,
}

impl LlmOrchestrator {
    pub fn new(config: ProviderConfig, system_prompt: String) -> Self {
        let client = OpenAiClient::new(config.base_url.clone(), config.api_key.clone());
        LlmOrchestrator {
            client,
            config,
            system_prompt,
            input_price_per_mtok: None,
            output_price_per_mtok: None,
        }
    }

    /// Attach metering prices (per 1M input/output tokens) so `estimated_usd`
    /// is real. Optional; without them, cost records a non-zero token count
    /// but `estimated_usd = 0` until a price map is wired.
    pub fn with_prices(mut self, input: f64, output: f64) -> Self {
        self.input_price_per_mtok = Some(input);
        self.output_price_per_mtok = Some(output);
        self
    }

    /// The planning instruction block describing the output contract. Kept as
    /// a standalone method so tests can assert the model is told the rules.
    fn planning_instructions(&self) -> String {
        // Enumerate the valid action vocabulary the model may emit (the gate
        // is the hard authority; this is the legible contract).
        let actions = concat!(
            "create_task, assign_task, start_task, complete_task,\n",
            "        request_review, review_task, commit_to_change_set,\n",
            "        raise_risk, resolve_risk, record_assumption, record_constraint,\n",
            "        record_opinion, record_fact, propose_decision, make_decision,\n",
            "        send_message, create_observation, block_task, block_task_on,\n",
            "        set_task_priority, decompose_task, propose_consultant, hire_agent,\n",
            "        propose_directive_change, create_directive, no_op"
        );
        format!(
            "You are the Project Manager for an autonomous software company.\n\
            \n\
            You act by emitting a list of VALID actions. Respond ONLY with a JSON \
            object of the form {{\"actions\": [...]}} where each action is one of the \
            following, serialized with an \"action\" tag:\n\
            {actions}\n\
            \n\
            Each action corresponds to a typed command the platform understands. \
            Choose actions that make progress toward the objective given the current \
            state. Every action is validated against policy afterwards, so only emit \
            actions that are legal in the current state (e.g. do not start a task \
            without a provisioned worktree, do not complete an unstarted task).\n\
            \n\
            Prefer a small, decisive set of actions. If there is genuinely nothing to \
            do, emit {{\"actions\": []}}. Never invent actions outside the list above.\n\
            \n\
            IMPORTANT: output ONLY the JSON object, no prose, no markdown fences."
        )
    }

    fn parse_actions(&self, content: &str) -> Result<Vec<PmAction>> {
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
    async fn plan(&self, context: &AgentContext, _cause: &Event) -> Result<PlanOutput> {
        let user_payload = serde_json::to_string_pretty(context)?;
        let req = ChatRequest {
            model: self.config.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: format!("{}\n\n{}", self.system_prompt, self.planning_instructions()),
                },
                ChatMessage {
                    role: "user".into(),
                    content: format!("Current operating context:\n{user_payload}"),
                },
            ],
            temperature: None,
            max_tokens: None,
            response_format: Some(serde_json::json!({"type": "json_object"})),
        };

        let started = std::time::Instant::now();
        let completion = self.client.chat(&req).await?;
        let latency_ms = started.elapsed().as_millis() as u64;

        let actions = self.parse_actions(&completion.content)?;

        let u = &completion.usage;
        let input_price = self.input_price_per_mtok.unwrap_or(0.0);
        let output_price = self.output_price_per_mtok.unwrap_or(0.0);
        let estimated_usd = (u.prompt_tokens as f64 * input_price
            + u.completion_tokens as f64 * output_price)
            / 1_000_000.0;

        let metering = CostMetering {
            agent_id: "pm".into(),
            task_id: None,
            model_tier: "standard".into(),
            model: Some(self.config.model.clone()),
            provider: Some(self.config.provider.clone()),
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            cache_read_input_tokens: u
                .prompt_tokens_details
                .as_ref()
                .map(|d| d.cached_tokens)
                .unwrap_or(0),
            cache_creation_input_tokens: 0,
            latency_ms,
            input_price_per_mtok: self.input_price_per_mtok,
            output_price_per_mtok: self.output_price_per_mtok,
            estimated_usd,
        };

        Ok(PlanOutput {
            actions: actions.into_iter().map(|a| ("pm".into(), a)).collect(),
            metering: Some(metering),
        })
    }
}
