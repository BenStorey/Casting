//! A minimal OpenAI-compatible `chat/completions` client.
//!
//! Deliberately thin: we speak ONE protocol so any OpenAI-compatible backend
//! (OpenRouter day-one, LiteLLM/vLLM/Ollama later) is a config switch, not a
//! code change. The provider's response `usage` is OpenAI-shaped, so metering
//! works unchanged across backends.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// One message in a chat/completions request.
#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// A request → `POST {base_url}/chat/completions`.
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Ask for a JSON object response (json_mode). Optional — some local
    /// backends ignore it; we parse defensively regardless.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,
}

/// The response `usage` — OpenAI-compatible token accounting (the same shape
/// OpenRouter and LiteLLM return), so `CostMetering` fills in unchanged.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Cache split, when the backend reports it (OpenRouter does via
    /// `prompt_tokens_details`).
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    /// EXACT USD cost reported by the provider for this call, when it does so.
    /// OpenRouter returns this as `usage.cost` in every response (the amount
    /// actually charged to the account) plus a `cost_details` breakdown.
    /// OpenAI and Anthropic direct APIs do NOT return cost — this stays None
    /// and metering falls back to a token-count × price estimate.
    #[serde(default)]
    pub cost: Option<f64>,
    #[serde(default)]
    pub cost_details: Option<CostDetails>,
}

/// The per-call cost breakdown some providers return alongside `usage.cost`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CostDetails {
    /// The actual cost charged by the upstream AI provider (OpenRouter passes
    /// this through; may differ from OpenRouter's own `usage.cost` when it
    /// applies a margin). Optional — many providers omit it.
    #[serde(default)]
    pub upstream_inference_cost: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PromptTokensDetails {
    /// Input tokens READ FROM the provider's prompt cache (a cache "hit").
    #[serde(default)]
    pub cached_tokens: u64,
    /// Input tokens WRITTEN to the provider's prompt cache (a "creation", NOT a
    /// hit — ~10x the read cost). Some OpenAI-compatible providers report this
    /// (e.g. via `prompt_tokens_details.cache_creation` or a similar field);
    /// most don't today, so it usually parses as 0 — but we thread it rather
    /// than hardcoding, so a reporting provider's write cost isn't dropped.
    #[serde(default, alias = "cache_creation", alias = "cache_creation_tokens")]
    pub cache_creation_tokens: u64,
}

/// The result of one chat/completions call: the model's text + usage.
#[derive(Debug, Clone)]
pub struct ChatCompletion {
    pub content: String,
    pub usage: Usage,
}

/// A thin `chat/completions` client for one provider endpoint (`base_url`).
#[derive(Debug, Clone)]
pub struct OpenAiClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl OpenAiClient {
    pub fn new(
        client: reqwest::Client,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        OpenAiClient {
            http: client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }

    /// POST `/chat/completions`, returning the first choice's text + usage.
    pub async fn chat(&self, req: &ChatRequest) -> Result<ChatCompletion> {
        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(req)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("LLM provider returned HTTP {status}: {body}");
        }

        // Parse generously: on failure keep usage best-effort, surface the raw
        // text so the caller can report/handle a malformed model reply.
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            choices: Vec<Choice>,
            #[serde(default)]
            usage: Option<Usage>,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: Message,
        }
        #[derive(Deserialize)]
        struct Message {
            #[serde(default)]
            content: Option<String>,
        }

        let body = resp.text().await?;
        let wire: Wire = serde_json::from_str(&body)?;
        let content = wire
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .unwrap_or_default();
        Ok(ChatCompletion {
            content,
            usage: wire.usage.unwrap_or_default(),
        })
    }
}

/// An Anthropic `/v1/messages` client, normalized into the SAME
/// [`ChatCompletion`] / [`Usage`] shape as [`OpenAiClient`] so the orchestrator
/// and cost metering stay provider-agnostic. Anthropic is NOT OpenAI-wire-
/// compatible (separate endpoint, `x-api-key` + `anthropic-version` headers,
/// system prompt as a top-level field, `max_tokens` required), so it needs its
/// own thin adapter rather than being a config switch.
#[derive(Debug, Clone)]
pub struct AnthropicClient {
    http: reqwest::Client,
    /// Base URL, e.g. `https://api.anthropic.com` (the client appends `/v1/messages`).
    base_url: String,
    api_key: String,
}

impl AnthropicClient {
    pub fn new(
        client: reqwest::Client,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        AnthropicClient {
            http: client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }

    /// POST `/v1/messages`, translating the OpenAI-shaped [`ChatRequest`] into
    /// Anthropic's wire format and the response back into [`ChatCompletion`].
    pub async fn chat(&self, req: &ChatRequest) -> Result<ChatCompletion> {
        let url = format!("{}/v1/messages", self.base_url);

        // Anthropic has no "system" role in the messages array — lift system
        // messages into the top-level `system` field; the rest become the
        // user/assistant conversation.
        let system = req
            .messages
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let messages: Vec<AnthropicMessage> = req
            .messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| AnthropicMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let mut body = serde_json::json!({
            "model": req.model,
            // Anthropic requires max_tokens (no server default).
            "max_tokens": req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "system": system,
            "messages": messages,
        });
        if let Some(t) = req.temperature {
            // Anthropic temperature is clamped to [0, 1].
            body["temperature"] = serde_json::json!(t.clamp(0.0, 1.0));
        }

        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic returned HTTP {status}: {text}");
        }

        let raw: serde_json::Value = resp.json().await?;

        // Content is an array of typed blocks; we want the text blocks joined.
        let content = match raw.get("content") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(blocks)) => blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        };

        // Anthropic reports the prompt as three DISJOINT buckets: uncached
        // input + cache reads + cache creations. Map them into the shared
        // Usage shape (total prompt + the cache split) so metering sees the
        // same fields OpenRouter reports.
        let usage = raw.get("usage").cloned().unwrap_or_default();
        let input = usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cache_read = usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cache_creation = usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output = usage
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        Ok(ChatCompletion {
            content,
            usage: Usage {
                prompt_tokens: input + cache_read + cache_creation,
                completion_tokens: output,
                prompt_tokens_details: Some(PromptTokensDetails {
                    cached_tokens: cache_read,
                    cache_creation_tokens: cache_creation,
                }),
                // Anthropic does not report USD cost.
                cost: None,
                cost_details: None,
            },
        })
    }
}

const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Anthropic requires an explicit `max_tokens`; this is the fallback when a
/// request carries none (the orchestrator's resolved bindings usually set one).
const DEFAULT_MAX_TOKENS: u32 = 8192;

#[derive(serde::Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

/// A provider-agnostic client seam: dispatch a [`ChatRequest`] to whichever
/// wire protocol the configured provider speaks, returning a normalized
/// [`ChatCompletion`]. OpenAI-compatible providers (OpenRouter, OpenAI,
/// LiteLLM, vLLM, Ollama) all use [`OpenAiClient`]; Anthropic uses
/// [`AnthropicClient`]. Everything downstream (orchestrator, advisor, cost
/// metering) never branches on provider.
#[derive(Debug, Clone)]
pub enum Client {
    OpenAi(OpenAiClient),
    Anthropic(AnthropicClient),
}

impl Client {
    /// Build the right client for a provider name. Anything that isn't
    /// Anthropic is treated as OpenAI-compatible.
    pub fn new(provider: &str, http: reqwest::Client, base_url: String, api_key: String) -> Self {
        if provider.eq_ignore_ascii_case("anthropic") {
            Client::Anthropic(AnthropicClient::new(http, base_url, api_key))
        } else {
            Client::OpenAi(OpenAiClient::new(http, base_url, api_key))
        }
    }

    /// One normalized `chat/completions`-shaped call across providers.
    pub async fn chat(&self, req: &ChatRequest) -> Result<ChatCompletion> {
        match self {
            Client::OpenAi(c) => c.chat(req).await,
            Client::Anthropic(c) => c.chat(req).await,
        }
    }
}
