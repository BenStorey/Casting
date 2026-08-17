//! Provider config for the D2 LLM client.
//!
//! The whole point: a provider is `(base_url, api_key, model, provider_name)`
//! over ONE OpenAI-compatible protocol. Swapping OpenRouter for a local LiteLLM
//! (or vLLM/Ollama) is changing a config value, never code.
//!
//! Config resolution order:
//!   1. Env vars (CAST_LLM_API_KEY, CAST_LLM_PROVIDER, etc.)
//!   2. Persisted config from `~/.casting/<slug>/config.json` (set via setup wizard)
//!   3. Defaults (openrouter + deepseek-v4-flash)

use crate::workspace::setup::read_config;
use anyhow::{Context, Result};
use std::path::Path;

/// The default OpenRouter model when none is configured via env or config.
const DEFAULT_OPENROUTER_MODEL: &str = "deepseek/deepseek-v4-flash-0731";

/// Resolved provider configuration. `base_url` already has the provider's
/// `/v1` prefix (chat/completions is appended by the client).
#[derive(Clone)]
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

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("api_key", &"***")
            .field("model", &self.model)
            .finish()
    }
}

/// Provider → default base_url map. `litellm` defaults to the conventional
/// localhost port (4000) but is overridable via `CAST_LLM_BASE_URL`.
pub fn default_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        "openai" => Some("https://api.openai.com/v1"),
        // Anthropic has no `/v1` here: the Anthropic client appends `/v1/messages`.
        "anthropic" => Some("https://api.anthropic.com"),
        "litellm" => Some("http://localhost:4000/v1"),
        _ => None,
    }
}

/// Resolve provider configuration from the environment or persisted config.
///
/// Resolution order:
///   1. Env vars (CAST_LLM_API_KEY, CAST_LLM_PROVIDER, CAST_LLM_MODEL, CAST_LLM_BASE_URL)
///   2. Persisted `~/.casting/<slug>/config.json` (api_key + defaults)
///
/// `state_dir` is the project's state-dir path (~/.casting/<slug>/), set during
/// `cast run`. Pass `None` to check env vars only.
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
                // Provider/model: explicit env override wins, else the value the
                // user chose at setup (provider + model), else the OpenRouter
                // defaults.
                let provider = std::env::var("CAST_LLM_PROVIDER")
                    .ok()
                    .filter(|p| !p.is_empty())
                    .or(cfg.provider.clone())
                    .unwrap_or_else(|| "openrouter".into());
                let model = std::env::var("CAST_LLM_MODEL")
                    .ok()
                    .filter(|m| !m.is_empty())
                    .or(cfg.model.clone())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Prove that a provider + model persisted by the setup wizard (in
    /// `~/.casting/<slug>/config.json`) flows through `from_env` into the resolved
    /// ProviderConfig — i.e. the setup → boot LLM wiring round-trips.
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("casting-cfg-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn persisted_provider_and_model_flow_through_from_env() {
        if std::env::var_os("CAST_LLM_API_KEY").is_some() {
            return; // env override would short-circuit; avoid flakiness
        }
        let dir = temp_dir("anthropic");
        std::fs::write(
            dir.join("config.json"),
            r#"{"name":"t","api_key":"sk-ant-test","provider":"anthropic","model":"claude-sonnet-4-5"}"#,
        )
        .unwrap();
        let cfg = from_env(Some(&dir)).unwrap().unwrap();
        assert_eq!(cfg.provider, "anthropic");
        assert_eq!(cfg.model, "claude-sonnet-4-5");
        assert_eq!(cfg.base_url, "https://api.anthropic.com");
        assert_eq!(cfg.api_key, "sk-ant-test");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persisted_defaults_to_openrouter_when_provider_unset() {
        if std::env::var_os("CAST_LLM_API_KEY").is_some() {
            return;
        }
        let dir = temp_dir("openrouter");
        std::fs::write(
            dir.join("config.json"),
            r#"{"name":"t","api_key":"sk-or-123"}"#,
        )
        .unwrap();
        let cfg = from_env(Some(&dir)).unwrap().unwrap();
        assert_eq!(cfg.provider, "openrouter");
        assert_eq!(cfg.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(cfg.model, DEFAULT_OPENROUTER_MODEL);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn openai_default_base_url_is_correct() {
        assert_eq!(
            default_base_url("openai"),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            default_base_url("anthropic"),
            Some("https://api.anthropic.com")
        );
    }
}
