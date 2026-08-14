//! Liveness watchdog — the "dead man's switch" (docs/plans/2026-08-13_harness-guards.md,
//! feature 3). The OTHER half of observability: durable execution recovers from
//! *crashes*; this detects when the system is *alive but not making progress*.
//!
//! Self-actuating, guardian-owned (owner message / dashboard notify is deferred
//! until messaging wiring): on a detected stall it issues a `WorkPaused` (by
//! `system`), which is the SAME resumable pause rail as the owner's `/api/pause`
//! and the shared halt gate in `guard.rs`. The cast halts itself; a human or the
//! owner resumes via `/api/resume`.
//!
//! Why a wall-clock monitor and not a cursor-gated reconciler pass: "no events
//! for N hours" is a TIME-based signal, and the reconciler's cadence only fires
//! when events keep arriving. A stuck system produces no new events, so nothing
//! would ever wake the reconciler. The watchdog runs on its own timer.
//!
//! Signals today (deterministic, LLM-free):
//!   - **RetryLoop** — the same error string repeats > `max_repeat_errors` times
//!     in the log (a stuck agent re-failing the same activity).
//!   - **NoProgress** — NO new events for `stall_hours` WHILE work is in flight
//!     (open Working/InReview tasks). Gated on in-flight work so a board that is
//!     legitimately idle (all done / waiting on the owner) is NOT misread as a
//!     stall.
//!
//! Spend-acceleration is a noted extension (would need a spend-rate baseline);
//! deliberately not built to keep the first cut small and false-positive-free.

use crate::actions::PmAction;
use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// Watchdog configuration. Defaults are conservative; env overrides let the
/// operator tune cadence. The monitor only runs when enabled (`CAST_WATCHDOG`).
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// How often the monitor polls the log (seconds).
    pub poll_secs: u64,
    /// No-new-events window before a stalled board is flagged (hours).
    pub stall_hours: u64,
    /// A single error string repeated more than this many times => retry loop.
    pub max_repeat_errors: usize,
}

impl Default for WatchConfig {
    fn default() -> Self {
        WatchConfig {
            poll_secs: 300,
            stall_hours: 24,
            max_repeat_errors: 3,
        }
    }
}

impl WatchConfig {
    /// Read from env (overrides), honouring the `CAST_WATCHDOG` enable flag.
    /// Disabled (returns None) unless `CAST_WATCHDOG=1`.
    pub fn from_env() -> Option<WatchConfig> {
        if std::env::var("CAST_WATCHDOG").ok().as_deref() != Some("1") {
            return None;
        }
        let mut c = WatchConfig::default();
        if let Ok(v) = std::env::var("CAST_WATCH_POLL_SECS") {
            c.poll_secs = v.parse().unwrap_or(c.poll_secs);
        }
        if let Ok(v) = std::env::var("CAST_WATCH_STALL_HOURS") {
            c.stall_hours = v.parse().unwrap_or(c.stall_hours);
        }
        if let Ok(v) = std::env::var("CAST_WATCH_MAX_REPEAT") {
            c.max_repeat_errors = v.parse().unwrap_or(c.max_repeat_errors);
        }
        Some(c)
    }
}

/// What the watchdog detected wrong.
#[derive(Debug, Clone, PartialEq)]
pub enum SignalKind {
    /// The same error repeated too many times (a retry loop).
    RetryLoop,
    /// No new events for too long while work was in flight.
    NoProgress,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub kind: SignalKind,
    pub detail: String,
}

/// A bounded, derived snapshot of liveness facts the detector reasons over.
/// Pure — built by [`scan`], reasoned by [`detect`], so tests can feed either.
#[derive(Debug, Clone, Default)]
pub struct WatchModel {
    /// Whether the company has started (any event exists at all).
    pub has_started: bool,
    /// Age of the newest event (0 when no events).
    pub last_event_age: Duration,
    /// Whether any task is mid-flight (Working / InReview) — "active" work.
    pub in_flight_work: bool,
    /// Repeated error strings -> their counts (from ActivityFailed events).
    pub repeated_errors: Vec<(String, usize)>,
}

/// How far back `scan` counts ActivityFailed errors for the retry-loop signal.
/// Recently-recurring errors are the real "stuck agent" signal; counting over
/// all history would let stale repeats accumulate and false-trigger.
const RETRY_ERROR_WINDOW: Duration = Duration::hours(1);

/// Build a [`WatchModel`] from the current log + wall clock.
pub fn scan(state: &AppState, now: DateTime<Utc>) -> WatchModel {
    let events = state
        .store
        .read_since(&state.project, 0)
        .unwrap_or_default();
    let has_started = !events.is_empty();
    let last_event_age = events
        .last()
        .map(|e| (now - e.timestamp).max(Duration::zero()))
        .unwrap_or_default();

    // Only failures within a recent window count toward a retry-loop signal.
    // Counting over ALL history would let a recurring-but-transient error from
    // weeks ago accumulate and false-trigger an auto-pause while current work
    // is healthy. Windowed counting keeps the "same error looping NOW" signal.
    let retry_window = now - RETRY_ERROR_WINDOW;

    let mut err_counts: HashMap<String, usize> = HashMap::new();
    for e in &events {
        if e.event_type == EventType::ActivityFailed && e.timestamp >= retry_window {
            if let Some(err) = e.data.get("error").and_then(|v| v.as_str()) {
                *err_counts.entry(err.to_string()).or_insert(0) += 1;
            }
        }
    }
    let mut repeated_errors: Vec<(String, usize)> = err_counts.into_iter().collect();
    repeated_errors.sort_by_key(|(_, n)| std::cmp::Reverse(*n));

    let proj = state.projection().unwrap_or_default();
    let in_flight_work = proj.tasks.iter().any(|t| {
        matches!(
            t.status,
            crate::projection::TaskStatus::Working | crate::projection::TaskStatus::InReview
        )
    });

    WatchModel {
        has_started,
        last_event_age,
        in_flight_work,
        repeated_errors,
    }
}

/// The decision core: given a derived model + config, is there a stall signal?
/// Pure and deterministic — easy to unit-test without clocks.
pub fn detect(config: &WatchConfig, model: &WatchModel) -> Option<Signal> {
    // Retry loop first (the loudest signal — a stuck agent re-failing).
    if let Some((err, n)) = model
        .repeated_errors
        .iter()
        .find(|(_, n)| *n > config.max_repeat_errors)
    {
        return Some(Signal {
            kind: SignalKind::RetryLoop,
            detail: format!(
                "watchdog: same error repeated {n}x (>{}): {}",
                config.max_repeat_errors,
                truncate(err, 120)
            ),
        });
    }
    // No progress: only when there IS in-flight work but the log has gone quiet.
    if model.has_started
        && model.in_flight_work
        && model.last_event_age > Duration::hours(config.stall_hours as i64)
    {
        return Some(Signal {
            kind: SignalKind::NoProgress,
            detail: format!(
                "watchdog: no events for >{}h while work is in flight (system may be frozen)",
                config.stall_hours
            ),
        });
    }
    None
}

/// The self-actuating action: scan, detect, and — if a stall is found and the
/// cast isn't already paused — issue a `WorkPaused` (by system). Returns the
/// pause reason, or `None` if nothing needed doing. Sync + idempotent (won't
/// re-pause if already paused), so the monitor and tests can call it directly.
pub fn audit(state: &AppState, config: &WatchConfig, now: DateTime<Utc>) -> Result<Option<String>> {
    let model = scan(state, now);
    let Some(sig) = detect(config, &model) else {
        return Ok(None);
    };
    let proj = state.projection()?;
    if crate::guard::is_paused(&proj) {
        // Already paused (owner or a previous watchdog fire) — leave it.
        return Ok(None);
    }
    let cause = last_or_boot(state);
    let action = PmAction::PauseWork {
        reason: sig.detail.clone(),
    };
    for ev in action.to_events(&state.project, "system", &cause, "watchdog") {
        state.append(ev)?;
    }
    eprintln!("[watchdog] auto-paused: {}", sig.detail);
    Ok(Some(sig.detail))
}

/// An event to act as the provenance "cause" for a watchdog-issued pause.
fn last_or_boot(state: &AppState) -> Event {
    state
        .store
        .read_since(&state.project, 0)
        .ok()
        .and_then(|v| v.into_iter().last())
        .unwrap_or_else(|| {
            Event::new(
                &state.project,
                Actor::System,
                EventType::ProjectCreated,
                Aggregate {
                    kind: "project".into(),
                    id: state.project.clone(),
                },
                serde_json::json!({}),
            )
        })
}

/// The long-running monitor: poll on the configured cadence and auto-pause on a
/// stall. Spawn as a background task from `cast run` (enabled via env).
pub async fn monitor(state: AppState, config: WatchConfig) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(config.poll_secs));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let now = Utc::now();
        if let Err(e) = audit(&state, &config, now) {
            eprintln!("[watchdog] audit error: {e:#}");
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n).collect();
        format!("{cut}…")
    }
}
