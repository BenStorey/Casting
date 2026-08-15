//! Per-actor model routing (docs/plans/2026-08-14_d2-routing-advisor-antithrash.md).
//!
//! The `ModelResolver` maps an ACTOR (agent id, "pm", "advisor") to the model
//! binding + persona it should use — so different roles run on different models
//! (Marcus→cheap, the Direction Advisor→premium, someone→local LiteLLM) instead
//! of everyone sharing the one env-configured model. This is what makes the
//! consultant `ModelConfig` (provider/model_id/base_url/cost_tier) real.

use crate::consultants::{ConsultantRegistry, ModelConfig};
use crate::llm::config::{default_base_url, ProviderConfig};

/// A resolved model binding + persona for one actor.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub config: ProviderConfig,
    /// The actor's persona (system prompt) to seed the call with.
    pub system_prompt: String,
    /// Sampling temperature, if the actor's binding declares one.
    pub temperature: Option<f32>,
    /// Max output tokens, if the actor's binding declares one.
    pub max_tokens: Option<u32>,
    /// Per-1M-token prices (input, output) for metering — derived from the
    /// actor's cost_tier (Standard default). Used to compute `estimated_usd`.
    pub input_price_per_mtok: f64,
    pub output_price_per_mtok: f64,
}

/// Default per-1M-token prices for a cost tier. These are CONFIG-SOURCED
/// defaults (the harness's "prices stay configured" stance — we do NOT query
/// a provider pricing API at runtime). Real spend for a given model may differ;
/// the point is that metering is non-zero so the budget breaker can work.
pub fn tier_prices(tier: crate::consultants::CostTier) -> (f64, f64) {
    use crate::consultants::CostTier::*;
    match tier {
        Budget => (0.15, 0.60),
        Standard => (1.00, 3.00),
        Premium => (5.00, 15.00),
    }
}

/// Maps an actor to its model binding + persona, from the consultant registry
/// overlaid on the env base config.
#[derive(Debug, Clone)]
pub struct ModelResolver {
    /// The base config (env): provider/base_url/api_key. The default for any
    /// actor with no consultant binding.
    base: ProviderConfig,
    /// The persona used for actors with no consultant system_prompt (e.g. "pm").
    /// Defaults to `"You are {actor}."` but is settable (e.g. a PM persona).
    default_persona: String,
    consultants: ConsultantRegistry,
}

impl ModelResolver {
    /// Build from the env base config + the loaded consultant registry.
    pub fn new(base: ProviderConfig, consultants: ConsultantRegistry) -> Self {
        ModelResolver {
            base,
            default_persona: String::new(),
            consultants,
        }
    }

    /// Set the fallback persona for actors with no consultant system_prompt.
    pub fn with_default_persona(mut self, persona: impl Into<String>) -> Self {
        self.default_persona = persona.into();
        self
    }

    /// The base env config's model (the "default" everyone would use).
    pub fn base(&self) -> &ProviderConfig {
        &self.base
    }

    /// Resolve the model + persona for an actor.
    ///
    /// Lookup order:
    /// 1. A consultant by agent id (e.g. "diego"). If it declares a model
    ///    binding (provider/model_id/base_url), use it — API key falls back to
    ///    env, base_url defaults via the provider map.
    /// 2. Otherwise the env base config.
    ///
    /// Persona: the consultant's system_prompt, else `default_persona` (else a
    /// generic `"You are {actor}."`).
    pub fn resolve(&self, actor: &str) -> ResolvedModel {
        let generic = format!("You are {actor}.");
        let fallback = if self.default_persona.is_empty() {
            generic
        } else {
            self.default_persona.clone()
        };
        match self.consultants.by_id(actor) {
            Some(consultant) => {
                let persona = consultant.system_prompt.clone().unwrap_or(fallback);
                let primary = consultant.primary_model();
                let temp = primary.and_then(|m| m.temperature);
                let max = primary.and_then(|m| m.max_tokens);
                let (in_price, out_price) =
                    tier_prices(primary.map(|m| m.cost_tier).unwrap_or_default());
                match primary.and_then(|m| model_from_consultant(m, &self.base)) {
                    Some(config) => ResolvedModel {
                        config,
                        system_prompt: persona,
                        temperature: temp,
                        max_tokens: max,
                        input_price_per_mtok: in_price,
                        output_price_per_mtok: out_price,
                    },
                    None => ResolvedModel {
                        config: self.base.clone(),
                        system_prompt: persona,
                        temperature: temp,
                        max_tokens: max,
                        input_price_per_mtok: in_price,
                        output_price_per_mtok: out_price,
                    },
                }
            }
            None => ResolvedModel {
                config: self.base.clone(),
                system_prompt: fallback,
                temperature: None,
                max_tokens: None,
                input_price_per_mtok: tier_prices(crate::consultants::CostTier::Standard).0,
                output_price_per_mtok: tier_prices(crate::consultants::CostTier::Standard).1,
            },
        }
    }

    /// Resolve the FULL ordered model chain for an actor, preferred first.
    /// The first entry is what `resolve` returns; each subsequent entry is a
    /// fallback to walk when the previous provider/model is unavailable
    /// (region block, rate limit, outage). D2's exec loop tries these in
    /// order. Empty when the actor has no binding (all paths fall back to the
    /// env base config as a single entry).
    pub fn resolve_chain(&self, actor: &str) -> Vec<ResolvedModel> {
        let generic = format!("You are {actor}.");
        let fallback = if self.default_persona.is_empty() {
            generic
        } else {
            self.default_persona.clone()
        };
        let Some(consultant) = self.consultants.by_id(actor) else {
            return vec![self.env_resolved(fallback)];
        };
        let persona = consultant.system_prompt.clone().unwrap_or(fallback);
        let mut chain: Vec<ResolvedModel> = consultant
            .model_chain()
            .iter()
            .filter_map(|m| {
                model_from_consultant(m, &self.base).map(|config| {
                    let (in_price, out_price) = tier_prices(m.cost_tier);
                    ResolvedModel {
                        config,
                        system_prompt: persona.clone(),
                        temperature: m.temperature,
                        max_tokens: m.max_tokens,
                        input_price_per_mtok: in_price,
                        output_price_per_mtok: out_price,
                    }
                })
            })
            .collect();
        if chain.is_empty() {
            // No usable binding declared — fall back to the env base once.
            chain.push(self.env_resolved(persona));
        }
        chain
    }

    /// A single `ResolvedModel` from the env base config + a persona.
    fn env_resolved(&self, persona: String) -> ResolvedModel {
        ResolvedModel {
            config: self.base.clone(),
            system_prompt: persona,
            temperature: None,
            max_tokens: None,
            input_price_per_mtok: tier_prices(crate::consultants::CostTier::Standard).0,
            output_price_per_mtok: tier_prices(crate::consultants::CostTier::Standard).1,
        }
    }
}

/// Turn a consultant's `ModelConfig` into a full `ProviderConfig`, given the
/// env base (for the API key + default base_url). Returns `None` when the
/// consultant declares no usable model id.
pub fn model_from_consultant(m: &ModelConfig, base: &ProviderConfig) -> Option<ProviderConfig> {
    let model_id = m.model_id.clone()?;
    let provider = m.provider.clone().unwrap_or_else(|| base.provider.clone());
    let base_url = m
        .base_url
        .clone()
        .or_else(|| default_base_url(&provider).map(|s| s.to_string()));
    Some(ProviderConfig {
        provider,
        base_url: base_url.unwrap_or_else(|| base.base_url.clone()),
        // Keys NEVER come from a consultant package — always the env base.
        api_key: base.api_key.clone(),
        model: model_id,
    })
}
