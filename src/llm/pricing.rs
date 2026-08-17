//! Per-model USD pricing resolution for cost metering.
//!
//! The metering pipeline needs a per-1M-token USD price for each model so it can
//! turn token counts into `estimated_usd` when the provider does NOT report the
//! exact cost itself (OpenRouter returns `usage.cost` — see `client.rs` — but
//! OpenAI and Anthropic direct APIs only return tokens).
//!
//! Prices resolve through a precedence ladder (first match wins):
//!   1. A local **override** table (`~/.casting/<slug>/prices.json`) — the "config, not
//!      code" escape hatch: pin a price, correct a stale one, or cover a local
//!      model (LiteLLM/Ollama) models.dev doesn't know.
//!   2. The **models.dev** open dataset (`https://models.dev/api.json`) cached
//!      to `~/.casting/<slug>/models_dev_cache.json` — the same community-maintained
//!      source Hermes uses, covering OpenAI/Anthropic/OpenRouter + 100+ other
//!      providers, with real input/output/cache_read/cache_write rates.
//!   3. A caller-supplied fallback (the consultant cost-tier prices) for
//!      unknown/local models.
//!
//! This keeps the price table auto-populated and current without hand-rolling a
//! per-model map in code, and it is fully provider-agnostic.

use crate::llm::client::Usage;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// USD per 1M tokens for one model. Cache rates are optional — when unknown,
/// cache tokens are priced at the (conservative) full input rate rather than
/// inventing a discount.
#[derive(Debug, Clone, Copy)]
pub struct CostPrices {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: Option<f64>,
    pub cache_write_per_mtok: Option<f64>,
}

impl CostPrices {
    /// A price set with only input/output rates (no cache split). Cache tokens
    /// are lumped into the input rate.
    pub fn from_halves(input: f64, output: f64) -> Self {
        CostPrices {
            input_per_mtok: input,
            output_per_mtok: output,
            cache_read_per_mtok: None,
            cache_write_per_mtok: None,
        }
    }

    /// Estimate USD for one call given the token split. When cache rates are
    /// known they're applied to their buckets; otherwise cache tokens are
    /// billed at the input rate (conservative — a cache hit is billed as if
    /// it were a fresh input).
    pub fn estimate_usd(
        &self,
        uncached_input: u64,
        cache_read: u64,
        cache_write: u64,
        output: u64,
    ) -> f64 {
        let input_cost = match (self.cache_read_per_mtok, self.cache_write_per_mtok) {
            (Some(cr), Some(cw)) => {
                uncached_input as f64 * self.input_per_mtok
                    + cache_read as f64 * cr
                    + cache_write as f64 * cw
            }
            _ => (uncached_input + cache_read + cache_write) as f64 * self.input_per_mtok,
        };
        (input_cost + output as f64 * self.output_per_mtok) / 1_000_000.0
    }
}

type PriceKey = (String, String); // (provider, model)

/// Resolves per-model prices from overrides + the cached models.dev dataset.
#[derive(Debug, Clone, Default)]
pub struct PricingResolver {
    overrides: HashMap<PriceKey, CostPrices>,
    models_dev: HashMap<PriceKey, CostPrices>,
}

impl PricingResolver {
    /// Load overrides (`~/.casting/<slug>/prices.json`) and the models.dev cache
    /// (`~/.casting/<slug>/models_dev_cache.json`) from the state dir, if present.
    /// Missing files are simply empty — callers fall back to tier prices.
    pub fn load(state_dir: Option<&Path>) -> Self {
        let mut r = PricingResolver::default();
        if let Some(dir) = state_dir {
            if let Ok(raw) = std::fs::read_to_string(dir.join("prices.json")) {
                if let Some(map) = parse_prices_json(&raw) {
                    r.overrides = map;
                }
            }
            if let Ok(raw) = std::fs::read_to_string(dir.join("models_dev_cache.json")) {
                r.models_dev = parse_models_dev(&raw);
            }
        }
        r
    }

    /// Resolve prices for a provider/model, or `None` if unknown (caller
    /// supplies a fallback).
    pub fn resolve(&self, provider: &str, model: &str) -> Option<CostPrices> {
        let key = (provider.to_string(), model.to_string());
        self.overrides
            .get(&key)
            .or_else(|| self.models_dev.get(&key))
            .copied()
    }

    /// Resolve prices, falling back to `fallback` when the model is unknown.
    pub fn resolve_or(&self, provider: &str, model: &str, fallback: CostPrices) -> CostPrices {
        self.resolve(provider, model).unwrap_or(fallback)
    }
}

/// Build a [`CostMetering`] for one call, choosing between the provider-reported
/// exact cost (OpenRouter `usage.cost`) and a token-count × price estimate.
///
/// `reported_cost_usd` is the authoritative provider cost when present; when it
/// is `None` (OpenAI/Anthropic direct), `estimated_usd` is computed from
/// `prices` with the cache split the provider reported.
#[allow(clippy::too_many_arguments)]
pub fn metering(
    agent_id: String,
    task_id: Option<String>,
    cost_class: String,
    model_tier: String,
    model: String,
    provider: String,
    u: &Usage,
    latency_ms: u64,
    prices: CostPrices,
    reported_cost_usd: Option<f64>,
) -> crate::runtime::orchestrator::CostMetering {
    let cache_read = u
        .prompt_tokens_details
        .as_ref()
        .map(|d| d.cached_tokens)
        .unwrap_or(0);
    let cache_write = u
        .prompt_tokens_details
        .as_ref()
        .map(|d| d.cache_creation_tokens)
        .unwrap_or(0);
    let uncached_input = u
        .prompt_tokens
        .saturating_sub(cache_read)
        .saturating_sub(cache_write);

    let estimated =
        prices.estimate_usd(uncached_input, cache_read, cache_write, u.completion_tokens);
    let (estimated_usd, cost_status) = match reported_cost_usd {
        Some(c) => (c, "actual".to_string()),
        None => (estimated, "estimated".to_string()),
    };

    crate::runtime::orchestrator::CostMetering {
        agent_id,
        task_id,
        cost_class,
        model_tier,
        model: Some(model),
        provider: Some(provider),
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        cache_read_input_tokens: cache_read,
        cache_creation_input_tokens: cache_write,
        latency_ms,
        input_price_per_mtok: Some(prices.input_per_mtok),
        output_price_per_mtok: Some(prices.output_per_mtok),
        cache_read_price_per_mtok: prices.cache_read_per_mtok,
        cache_write_price_per_mtok: prices.cache_write_per_mtok,
        reported_cost_usd,
        cost_status,
        estimated_usd,
    }
}

/// Parse a `~/.casting/<slug>/prices.json` override table: `{ "provider/model": { ... } }`.
pub fn parse_prices_json(raw: &str) -> Option<HashMap<PriceKey, CostPrices>> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = v.as_object()?;
    let mut out = HashMap::new();
    for (key, val) in obj {
        let (provider, model) = key.split_once('/')?;
        let cost = val.as_object()?;
        let input = cost.get("input").and_then(|x| x.as_f64())?;
        let output = cost.get("output").and_then(|x| x.as_f64())?;
        out.insert(
            (provider.to_string(), model.to_string()),
            CostPrices {
                input_per_mtok: input,
                output_per_mtok: output,
                cache_read_per_mtok: cost.get("cache_read").and_then(|x| x.as_f64()),
                cache_write_per_mtok: cost.get("cache_write").and_then(|x| x.as_f64()),
            },
        );
    }
    Some(out)
}

/// Parse the raw models.dev dataset (`api.json`) into a (provider, model) →
/// prices map. models.dev shapes it as `{ "<provider>": { "models": { "<id>":
/// { "cost": { "input", "output", "cache_read", "cache_write" } } } } }`.
pub fn parse_models_dev(raw: &str) -> HashMap<PriceKey, CostPrices> {
    let mut out = HashMap::new();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return out;
    };
    let Some(providers) = v.as_object() else {
        return out;
    };
    for (provider, pv) in providers {
        let Some(models) = pv.get("models").and_then(|m| m.as_object()) else {
            continue;
        };
        for (model, mval) in models {
            let Some(cost) = mval.get("cost") else {
                continue;
            };
            let input = cost.get("input").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let output = cost.get("output").and_then(|x| x.as_f64()).unwrap_or(0.0);
            if input <= 0.0 && output <= 0.0 {
                continue; // no pricing published — skip
            }
            out.insert(
                (provider.clone(), model.clone()),
                CostPrices {
                    input_per_mtok: input,
                    output_per_mtok: output,
                    cache_read_per_mtok: cost.get("cache_read").and_then(|x| x.as_f64()),
                    cache_write_per_mtok: cost.get("cache_write").and_then(|x| x.as_f64()),
                },
            );
        }
    }
    out
}

/// Fetch the models.dev dataset and write it verbatim to the state-dir cache,
/// so the resolver has prices without a hardcoded table. Best-effort: callers
/// ignore errors (tier prices cover the gap). Only the raw JSON is cached and
/// re-parsed on load — the dataset IS the cache, matching Hermes's approach.
/// A cache younger than [`MODELS_DEV_TTL`] is left untouched (no network hit
/// every boot).
pub async fn fetch_models_dev(state_dir: &Path, http: &reqwest::Client) -> Result<()> {
    let cache_path = state_dir.join("models_dev_cache.json");
    if let Ok(meta) = std::fs::metadata(&cache_path) {
        if let Ok(modified) = meta.modified() {
            if let Ok(elapsed) = modified.elapsed() {
                if elapsed < MODELS_DEV_TTL {
                    return Ok(()); // fresh enough — keep existing cache
                }
            }
        }
    }
    let resp = http
        .get("https://models.dev/api.json")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("models.dev returned HTTP {}", resp.status());
    }
    let text = resp.text().await?;
    // Sanity: must parse to a provider map before we persist anything.
    if parse_models_dev(&text).is_empty() {
        anyhow::bail!("models.dev response parsed to no pricing entries");
    }
    let path = state_dir.join("models_dev_cache.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, text.as_bytes())?;
    Ok(())
}

/// How long a cached models.dev dataset is considered fresh (24h).
const MODELS_DEV_TTL: std::time::Duration = std::time::Duration::from_secs(60 * 60 * 24);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::client::{PromptTokensDetails, Usage};

    #[test]
    fn estimate_usd_bills_cache_split_when_known() {
        // gpt-4o: $2.5 in / $10 out / $1.25 cache read.
        let p = CostPrices {
            input_per_mtok: 2.5,
            output_per_mtok: 10.0,
            cache_read_per_mtok: Some(1.25),
            cache_write_per_mtok: Some(3.125),
        };
        // 900 uncached + 100 cache read in, 100 out.
        let usd = p.estimate_usd(900, 100, 0, 100);
        let expected = (900.0 * 2.5 + 100.0 * 1.25 + 100.0 * 10.0) / 1_000_000.0;
        assert!((usd - expected).abs() < 1e-12);
    }

    #[test]
    fn estimate_usd_lumps_cache_at_input_when_unknown() {
        let p = CostPrices::from_halves(2.5, 10.0);
        // Cache rates unknown -> cache tokens billed at input rate.
        let usd = p.estimate_usd(900, 100, 0, 100);
        let expected = (1000.0 * 2.5 + 100.0 * 10.0) / 1_000_000.0;
        assert!((usd - expected).abs() < 1e-12);
    }

    #[test]
    fn metering_prefers_reported_cost() {
        let u = Usage {
            prompt_tokens: 1000,
            completion_tokens: 100,
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: 100,
                cache_creation_tokens: 0,
            }),
            cost: Some(0.004),
            cost_details: None,
        };
        let m = metering(
            "mei".into(),
            None,
            "pm_overhead".into(),
            "standard".into(),
            "gpt-4o".into(),
            "openai".into(),
            &u,
            50,
            CostPrices::from_halves(2.5, 10.0),
            u.cost,
        );
        assert_eq!(m.cost_status, "actual");
        assert_eq!(m.estimated_usd, 0.004);
        assert_eq!(m.reported_cost_usd, Some(0.004));
        assert_eq!(m.cache_read_input_tokens, 100);
    }

    #[test]
    fn metering_estimates_when_no_reported_cost() {
        let u = Usage {
            prompt_tokens: 1000,
            completion_tokens: 100,
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: 100,
                cache_creation_tokens: 0,
            }),
            cost: None,
            cost_details: None,
        };
        let m = metering(
            "diego".into(),
            None,
            "implementation".into(),
            "standard".into(),
            "gpt-4o".into(),
            "openai".into(),
            &u,
            50,
            CostPrices::from_halves(2.5, 10.0),
            None,
        );
        assert_eq!(m.cost_status, "estimated");
        // 900 uncached * 2.5 + 100 cached * 2.5 (unknown cache rate -> lumped)
        // + 100 * 10, all / 1e6.
        assert!((m.estimated_usd - (1000.0 * 2.5 + 100.0 * 10.0) / 1e6).abs() < 1e-12);
        assert_eq!(m.reported_cost_usd, None);
    }

    #[test]
    fn parse_models_dev_inverts_provider_models() {
        let raw = r#"{
            "openai": { "models": {
                "gpt-4o": { "cost": { "input": 2.5, "output": 10, "cache_read": 1.25 } },
                "gpt-4o-mini": { "cost": { "input": 0.15, "output": 0.60 } }
            } },
            "anthropic": { "models": {
                "claude-sonnet-4-5": { "cost": { "input": 3.0, "output": 15.0, "cache_read": 0.30, "cache_write": 3.75 } }
            } },
            "misc": { "models": { "no-cost": { "cost": {} } } }
        }"#;
        let map = parse_models_dev(raw);
        let gpt4o = map.get(&("openai".into(), "gpt-4o".into())).unwrap();
        assert_eq!(gpt4o.input_per_mtok, 2.5);
        assert_eq!(gpt4o.cache_read_per_mtok, Some(1.25));
        let sonnet = map
            .get(&("anthropic".into(), "claude-sonnet-4-5".into()))
            .unwrap();
        assert_eq!(sonnet.cache_write_per_mtok, Some(3.75));
        // No-pricing model with all-zero cost is skipped.
        assert!(!map.contains_key(&("misc".into(), "no-cost".into())));
    }

    #[test]
    fn parse_prices_json_override() {
        let raw = r#"{
            "anthropic/claude-sonnet-4-5": { "input": 3.0, "output": 15.0, "cache_read": 0.30 }
        }"#;
        let map = parse_prices_json(raw).unwrap();
        let p = map
            .get(&("anthropic".into(), "claude-sonnet-4-5".into()))
            .unwrap();
        assert_eq!(p.output_per_mtok, 15.0);
        assert_eq!(p.cache_read_per_mtok, Some(0.30));
    }

    #[test]
    fn resolver_prefers_override_over_models_dev() {
        let mut r = PricingResolver::default();
        r.overrides.insert(
            ("openai".into(), "gpt-4o".into()),
            CostPrices::from_halves(9.9, 19.9),
        );
        r.models_dev.insert(
            ("openai".into(), "gpt-4o".into()),
            CostPrices::from_halves(2.5, 10.0),
        );
        assert_eq!(r.resolve("openai", "gpt-4o").unwrap().input_per_mtok, 9.9);
        assert_eq!(
            r.resolve_or("unknown", "model", CostPrices::from_halves(1.0, 2.0))
                .input_per_mtok,
            1.0
        );
    }
}
