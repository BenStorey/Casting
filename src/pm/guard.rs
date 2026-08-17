//! Harness guards — the hard safety rails that sit OUTSIDE the PM's control
//! (docs/plans/2026-08-13_harness-guards.md, HARNESS #6/#9 + liveness).
//!
//! Design stance: **the PM *optimizes*; the guard *refuses*.** The PM is an
//! agent — it can be confused, compromised, or stuck. The rails below are pure
//! deterministic projections + gate checks, so they hold even when the PM is
//! wrong. They are LLM-free and event-sourced (the event log is still the only
//! authority; spend and pause are derived state, never a side ledger).
//!
//! Two ORTHOGONAL mechanisms:
//!
//! 1. **Budget** — derived straight from `proj.spend` (which never decreases),
//!    so a halt is a permanent, always-recomputed state. NOT resumable via
//!    `ResumeWork` (spend doesn't go down); only a higher budget limit
//!    un-halts it. Set by the director via `BudgetSet`.
//! 2. **Pause** — a resumable flag (`WorkPaused`/`WorkResumed`) used by the
//!    owner or by the liveness watchdog to stop all side-effecting work.

use crate::projection::Projection;
use serde::{Deserialize, Serialize};

/// Owner-set token budget. `warn_at` is the fraction of `limit_usd` at which
/// the PM is warned (default 0.80); at `limit_usd` all LLM calls are refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub limit_usd: f64,
    #[serde(default = "default_warn_at")]
    pub warn_at: f64,
}

fn default_warn_at() -> f64 {
    0.80
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            limit_usd: 0.0,
            warn_at: 0.80,
        }
    }
}

/// Why work is currently paused (owner- or watchdog-initiated; resumable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PauseInfo {
    pub reason: String,
    pub by: String,
    pub at: String,
}

/// The budget phase, derived from spend vs the configured budget.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetStatus {
    /// No budget configured — no automatic spend guard.
    Disabled,
    /// Spend below the warn threshold.
    Ok,
    /// spend >= warn_at * limit; the PM should be warned, calls still allowed.
    Warn { fraction: f64 },
    /// spend >= limit; **refuse all LLM dispatch** (the hard breaker).
    Halted { fraction: f64 },
}

impl BudgetStatus {
    /// Human-readable label for surfacing (`/api/model` guards view).
    pub fn label(&self) -> &'static str {
        match self {
            BudgetStatus::Disabled => "disabled",
            BudgetStatus::Ok => "ok",
            BudgetStatus::Warn { .. } => "warn",
            BudgetStatus::Halted { .. } => "halted",
        }
    }
}

/// Current spend as a fraction of the budget (0.0 when unset/invalid).
pub fn budget_fraction(proj: &Projection) -> f64 {
    match &proj.budget {
        Some(b) if b.limit_usd > 0.0 => proj.total_spend_usd() / b.limit_usd,
        _ => 0.0,
    }
}

/// Returns `true` when a budget limit is actually configured and active.
///
/// A budget of `Disabled` (i.e. `proj.budget` is `None` or `limit_usd <= 0.0`)
/// means no LLM dispatch is allowed — the gate refuses all calls until a
/// budget is configured via `BudgetSet`. This is intentional: a developer
/// installing the tool and wiring up an API key should not be able to
/// accidentally burn unbounded spend before setting a cap.
pub fn budget_is_configured(proj: &Projection) -> bool {
    proj.budget.is_some() && proj.budget.as_ref().unwrap().limit_usd > 0.0
}

/// Derive the budget phase from the projection. This is the hard breaker's
/// check — deterministic and always recomputed from spend (the event log).
pub fn budget_status(proj: &Projection) -> BudgetStatus {
    let Some(b) = &proj.budget else {
        return BudgetStatus::Disabled;
    };
    if b.limit_usd <= 0.0 {
        return BudgetStatus::Disabled;
    }
    let fraction = proj.total_spend_usd() / b.limit_usd;
    if fraction >= 1.0 {
        BudgetStatus::Halted { fraction }
    } else if fraction >= b.warn_at {
        BudgetStatus::Warn { fraction }
    } else {
        BudgetStatus::Ok
    }
}

/// True when a resumable pause is currently in effect.
pub fn is_paused(proj: &Projection) -> bool {
    proj.paused.is_some()
}

/// The gate every LLM/config-dispatch point consults BEFORE doing work.
/// Refuses when work is paused, the budget is not configured, or the budget
/// is exhausted (the hard breaker). `Err` carries a human-readable reason.
pub fn llm_dispatch_allowed(proj: &Projection) -> Result<(), String> {
    if let Some(p) = &proj.paused {
        return Err(format!("work paused: {} (by {})", p.reason, p.by));
    }
    match budget_status(proj) {
        BudgetStatus::Disabled => {
            return Err(
                "budget not configured: set a budget via the web UI or POST /api/budget \
                 before dispatching LLM calls"
                    .into(),
            );
        }
        BudgetStatus::Halted { .. } => {
            return Err(format!(
                "budget exhausted: spend ${:.2} >= limit ${:.2}; raise the budget to resume",
                proj.total_spend_usd(),
                proj.budget.as_ref().map(|b| b.limit_usd).unwrap_or(0.0),
            ));
        }
        _ => {}
    }
    Ok(())
}
