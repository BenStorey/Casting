//! Provider config for the D2 LLM client.
//!
//! The whole point: a provider is `(base_url, api_key, model, provider_name)`
//! over ONE OpenAI-compatible protocol. Swapping OpenRouter for a local LiteLLM
//! (or vLLM/Ollama) is changing a config value, never code.

use anyhow::{Context, Result};

/// Resolved provider configuration. `base_url` already has the provider's
/// `/v1` prefix (chat/completions is appended by the client).
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// The provider name for metering/audit (free string, e.g. "openrouter").
    pub provider: String,
    /// OpenAI-compatible base URL, e.g. "https://openrouter.ai/api/v1".
    pub base_url: String,
    /// Bearer API key. Never logged.
    pub api_key: String,
    /// Model id, e.g. "deepseek/deepseek-v4-flash-0731".
    pub model: String,
}

/// Provider → default base_url map. `litellm` defaults to the conventional
/// localhost port (4000) but is overridable via `CAST_LLM_BASE_URL`.
pub fn default_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        "litellm" => Some("http://localhost:4000/v1"),
        _ => None,
    }
}

/// Resolve provider configuration from the environment.
///
/// Day one requires an API key (OpenRouter). A bare model+provider with no key
/// yields `None` — the deterministic scripted PM stays the default until the
/// owner configures the LLM.
pub fn from_env() -> Result<Option<ProviderConfig>> {
    // Requiring an API key is the "is LLM wiring on?" signal.
    let api_key = match std::env::var("CAST_LLM_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => return Ok(None),
    };

    let provider = std::env::var("CAST_LLM_PROVIDER").unwrap_or_else(|_| "openrouter".into());
    let model = std::env::var("CAST_LLM_MODEL")
        .context("CAST_LLM_MODEL must be set with CAST_LLM_API_KEY")?;
    let base_url = match std::env::var("CAST_LLM_BASE_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => default_base_url(&provider)
            .with_context(|| format!("unknown LLM provider '{provider}' (set CAST_LLM_BASE_URL)"))?
            .to_string(),
    };

    Ok(Some(ProviderConfig {
        provider,
        base_url,
        api_key,
        model,
    }))
}
