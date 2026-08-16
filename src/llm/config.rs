//! Provider config for the D2 LLM client.
//!
//! The whole point: a provider is `(base_url, api_key, model, provider_name)`
//! over ONE OpenAI-compatible protocol. Swapping OpenRouter for a local LiteLLM
//! (or vLLM/Ollama) is changing a config value, never code.
//!
//! Config resolution order:
//!   1. Env vars (CAST_LLM_API_KEY, CAST_LLM_PROVIDER, etc.)
//!   2. Persisted config from `.casting/config.json` (set via setup wizard)
//!   3. Defaults (openrouter + deepseek-v4-flash)

use crate::workspace::setup::read_config;
use anyhow::{Context, Result};
use std::path::Path;

/// The default OpenRouter model when none is configured via env or config.
const DEFAULT_OPENROUTER_MODEL: &str = "deepseek/deepseek-v4-flash-0731";

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

/// Resolve provider configuration from the environment or persisted config.
///
/// Resolution order:
///   1. Env vars (CAST_LLM_API_KEY, CAST_LLM_PROVIDER, CAST_LLM_MODEL, CAST_LLM_BASE_URL)
///   2. Persisted `.casting/config.json` (api_key + defaults)
///
/// `state_dir` is the `.casting/` directory path, set during `cast run`.
/// Pass `None` to check env vars only.
///
/// When the persisted path finds an API key but no model/provider, defaults
/// are used (openrouter + deepseek/deepseek-v4-flash-0731).
pub fn from_env(state_dir: Option<&Path>) -> Result<Option<ProviderConfig>> {
    // 1. Try env vars first.
    if let Ok(key) = std::env::var("CAST_LLM_API_KEY") {
        if !key.is_empty() {
            return Ok(Some(from_env_inner(key)?));
        }
    }

    // 2. Fall back to persisted config.
    if let Some(dir) = state_dir {
        if let Some(cfg) = read_config(dir) {
            if let Some(api_key) = cfg.api_key.filter(|k| !k.is_empty()) {
                let provider =
                    std::env::var("CAST_LLM_PROVIDER").unwrap_or_else(|_| "openrouter".into());
                let model = std::env::var("CAST_LLM_MODEL")
                    .ok()
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| DEFAULT_OPENROUTER_MODEL.to_string());
                let base_url = match std::env::var("CAST_LLM_BASE_URL") {
                    Ok(u) if !u.is_empty() => u,
                    _ => default_base_url(&provider)
                        .unwrap_or("https://openrouter.ai/api/v1")
                        .to_string(),
                };
                return Ok(Some(ProviderConfig {
                    provider,
                    base_url,
                    api_key,
                    model,
                }));
            }
        }
    }

    Ok(None)
}

/// Build a `ProviderConfig` from env vars, assuming the API key is already set.
fn from_env_inner(api_key: String) -> Result<ProviderConfig> {
    let provider = std::env::var("CAST_LLM_PROVIDER").unwrap_or_else(|_| "openrouter".into());
    let model = std::env::var("CAST_LLM_MODEL")
        .context("CAST_LLM_MODEL must be set with CAST_LLM_API_KEY")?;
    let base_url = match std::env::var("CAST_LLM_BASE_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => default_base_url(&provider)
            .with_context(|| format!("unknown LLM provider '{provider}' (set CAST_LLM_BASE_URL)"))?
            .to_string(),
    };
    Ok(ProviderConfig {
        provider,
        base_url,
        api_key,
        model,
    })
}
