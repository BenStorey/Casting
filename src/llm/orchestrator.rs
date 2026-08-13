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
        // Enumerate the valid action vocabulary the model may emit, WITH the
        // exact required fields — the model must emit valid typed commands or
        // the parse fails. The gate is the hard authority; this is the legible
        // contract that makes the model's output parse.
        let actions = r#"- create_task:      {"action":"create_task","id":str,"title":str,"kind":str}
- assign_task:      {"action":"assign_task","task_id":str,"assignee":str}
- start_task:       {"action":"start_task","task_id":str}
- complete_task:    {"action":"complete_task","task_id":str,"result":str}
- request_review:   {"action":"request_review","task_id":str,"reviewer":str}
- review_task:      {"action":"review_task","task_id":str,"approved":bool,"note":str|null}
- commit_to_change_set: {"action":"commit_to_change_set","task_id":str,"message":str}
- raise_risk:       {"action":"raise_risk","id":str,"subject":str,"severity":str}
- resolve_risk:     {"action":"resolve_risk","risk_id":str,"status":"open"|"materialized"|"resolved"}
- record_assumption: {"action":"record_assumption","id":str,"body":str}
- record_constraint: {"action":"record_constraint","id":str,"body":str}
- record_opinion:   {"action":"record_opinion","id":str,"subject":str,"category":str,"statement":str,"supersedes":str|null}
- record_fact:      {"action":"record_fact","id":str,"kind":str,"statement":str}
- propose_decision: {"action":"propose_decision","id":str,"subject":str,"options":{...},"recommendation":str,"class":"internal_implementation"|"internal_refactor"|"add_consultant"|"testing_library"|"security_critical"|"production_deployment"|"product_requirement","involvement":"pm"|"ask"|"never"|"notify"}
- make_decision:    {"action":"make_decision","decision_id":str,"approved":bool,"note":str|null}
- send_message:     {"action":"send_message","to":str,"body":str}
- create_observation: {"action":"create_observation","id":str,"severity":str,"subject":str,"body":str,"pm_action_required":bool}
- block_task:       {"action":"block_task","task_id":str,"reason":str}
- set_task_priority: {"action":"set_task_priority","task_id":str,"priority":"low"|"medium"|"high"|"critical"}
- block_task_on:    {"action":"block_task_on","task_id":str,"blocking_task_id":str,"required_state":"backlog"|"working"|"in_review"|"blocked"|"done"}
- propose_consultant: {"action":"propose_consultant","id":str,"subject":str,"role_id":str,"involvement":"pm"|"ask"|"never"}
- no_op:            {"action":"no_op"}
"#;

        format!(
            "You are the Project Manager for an autonomous software company.\n\
            \n\
            You act by emitting a list of VALID actions. Respond ONLY with a JSON \
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
    async fn plan(&self, context: &AgentContext, cause: &Event) -> Result<PlanOutput> {
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
            model: self.config.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: format!("{}\n\n{}", self.system_prompt, self.planning_instructions()),
                },
                ChatMessage {
                    role: "user".into(),
                    content: format!(
                        "The owner just said: \"{ask}\"\n\n\
                         Current operating context:\n{user_payload}"
                    ),
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
