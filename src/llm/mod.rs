//! D2 — the real LLM provider client + orchestrator (docs/plans/2026-08-13_d2-llm-wiring.md).
//!
//! Day one: OpenRouter. Later: any OpenAI-compatible backend (LiteLLM, vLLM,
//! Ollama, ...) — because we speak ONE protocol (`POST /v1/chat/completions`),
//! the provider is CONFIG (`base_url` + `api_key` + `model`), never code.
//!
//! - `config`: [`ProviderConfig`] — the (base_url, api_key, model, provider)
//!   tuple + env resolution + the provider→base_url map.
//! - `client`: [`OpenAiClient`] — a minimal chat/completions client returning
//!   content + usage (OpenAI-compatible, so metering just works).
//! - `orchestrator`: [`LlmOrchestrator`] — implements the [`crate::runtime::orchestrator::Orchestrator`]
//!   seam: prompt build → call → `PmAction` parse → `CostMetering`.

pub mod advisor;
pub mod client;
pub mod config;
pub mod orchestrator;
pub mod pricing;
pub mod routing;

pub use advisor::{
    advisor_reply, advisor_summarize, advisor_summarize_deterministic, AdvisorOutcome,
};
pub use client::{AnthropicClient, ChatMessage, ChatRequest, Client, OpenAiClient};
pub use config::ProviderConfig;
pub use orchestrator::LlmOrchestrator;
pub use pricing::{fetch_models_dev, metering, CostPrices, PricingResolver};
pub use routing::{model_from_consultant, ModelResolver, ResolvedModel};
