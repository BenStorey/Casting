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
