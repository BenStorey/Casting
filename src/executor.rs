//! Durable, crash-safe side-effecting execution (durability first PR).
//!
//! The event log is the ONLY authority; process memory is disposable. The one
//! place a crash can do real harm is re-running a *side effect* — an LLM call,
//! a `git push`, a shell command — that started but whose result never landed
//! because the server died mid-activity. State can't be lost (the PM resumes
//! from its durable cursor), but physical work can be re-run.
//!
//! Mechanism: every discrete side-effecting action carries a STABLE activity id
//! (e.g. `task-7-llm-call-3`). We record intent as an `ActivityScheduled` event
//! BEFORE doing any work. When execution finishes we append `ActivityCompleted`
//! (or `ActivityFailed`). The idempotency guarantee: before doing physical
//! work, check the log — if an `ActivityCompleted` already exists for this id,
//! skip execution entirely.
//!
//! After a crash, [`redispatch_inflight`] re-runs anything that was scheduled
//! but never finished; the guard makes the re-run safe. The PM loop resumes
//! from its durable cursor and sees the appended events via the normal
//! broadcast — the executor never calls `pm.evaluate` directly (that would
//! couple the executor to the PM and fight the cursor model).
//!
//! Design rules (from docs/plans/2026-08-13_durability-first-pr.md):
//! - No `workflow_id`/`event_number`: the task `aggregate` + global `sequence`
//!   already give deterministic ordering. A "workflow" == a task aggregate id.
//! - No parallel workflow state machine: crash state is already derivable from
//!   the projection + `graph.rs`.
//! - Large results are stored as a `result_ref` (path/object id), never inline.

use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The side effect to perform. The executor is the ONLY place real external
/// work happens; everything else is a deterministic reducer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ActivityKind {
    /// An LLM/OpenRouter call (the D2 seam — unplugged today, guard ready).
    LlmCall { prompt: String },
    /// A git push to a branch.
    GitPush { branch: String },
    /// An arbitrary shell command.
    Shell { cmd: String },
    /// No external work — result computed inline. Never needs re-dispatch.
    Inline,
}

/// A discrete, stable-identified side-effecting action.
///
/// `id` is the idempotency key (e.g. `task-7-llm-call-3`); `target_id` is the
/// task aggregate it belongs to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    pub id: String,
    pub target_id: String,
    pub kind: ActivityKind,
}

/// Result of running an activity. `result_ref` points at a stored artifact for
/// large payloads (never inline).
#[derive(Debug, Clone, Default)]
pub struct ActivityResult {
    pub result_ref: Option<String>,
}

/// Injected side-effect executor. A trait so tests stay deterministic and the
/// real LLM/git/shell impls plug in later. The idempotency guard is ORTHOGONAL
/// to this — it never depends on which runner is wired.
pub trait ActivityRunner: Send + Sync {
    fn run(&self, activity: &Activity) -> Result<ActivityResult>;
}

/// A runner that can only do inline (derived) work. Any real external kind
/// fails loudly rather than silently fake-completing. This is the safe default
/// until D2 (LLM) / git / shell runners are wired.
pub struct NoopRunner;

impl ActivityRunner for NoopRunner {
    fn run(&self, activity: &Activity) -> Result<ActivityResult> {
        match &activity.kind {
            ActivityKind::Inline => Ok(ActivityResult::default()),
            other => Err(anyhow!(
                "no runner wired for {:?} (D2/git executor not connected yet)",
                other
            )),
        }
    }
}

/// True if the log already contains an `ActivityCompleted` for `activity_id`.
/// This is the idempotency check — the whole durability mechanism.
pub fn has_completed(state: &AppState, activity_id: &str) -> Result<bool> {
    let events = state.store.read_since(&state.project, 0)?;
    Ok(events.iter().any(|e| {
        e.event_type == EventType::ActivityCompleted
            && e.data.get("id").and_then(|v| v.as_str()) == Some(activity_id)
    }))
}

/// True if the log already contains an `ActivityFailed` for `activity_id`.
pub fn has_failed(state: &AppState, activity_id: &str) -> Result<bool> {
    let events = state.store.read_since(&state.project, 0)?;
    Ok(events.iter().any(|e| {
        e.event_type == EventType::ActivityFailed
            && e.data.get("id").and_then(|v| v.as_str()) == Some(activity_id)
    }))
}

/// Was this activity ever terminated (completed OR failed) in the log?
fn is_terminated(state: &AppState, activity_id: &str) -> Result<bool> {
    Ok(has_completed(state, activity_id)? || has_failed(state, activity_id)?)
}

/// Append an `ActivityScheduled` event — the durable *intent* record. Call once
/// before the first execution of an activity. The event's `data` carries the
/// full `Activity` so a crash-triggered re-dispatch can reconstruct it.
pub fn schedule(state: &AppState, actor: Actor, activity: &Activity) -> Result<()> {
    state.append(build_event(
        state,
        actor,
        EventType::ActivityScheduled,
        activity,
        json!({}),
    ))?;
    Ok(())
}

/// Run one activity with the idempotency guard. If `ActivityCompleted` already
/// exists for this id, skip and return `Ok` (no re-run). Otherwise run the
/// side effect, then append `ActivityCompleted` (or `ActivityFailed` on error)
/// so the broadcast wakes the PM loop.
pub fn execute(
    state: &AppState,
    runner: &dyn ActivityRunner,
    actor: Actor,
    activity: &Activity,
) -> Result<ActivityResult> {
    // Harness gate (2026-08-13, guard.rs): refuse NEW side-effecting work while
    // work is paused or the budget is exhausted. Inline (derived) work touches
    // no external resource and is always allowed. Fail-closed: mark the
    // activity failed so it won't auto-re-dispatch while the guard still blocks.
    if !matches!(activity.kind, ActivityKind::Inline) {
        let proj = state.projection()?;
        if let Err(reason) = crate::guard::llm_dispatch_allowed(&proj) {
            let message = format!("guard blocked {}: {reason}", activity.id);
            state.append(build_event(
                state,
                actor,
                EventType::ActivityFailed,
                activity,
                json!({ "error": message }),
            ))?;
            return Err(anyhow!("{message}"));
        }
    }
    // Idempotency guard: already done before a crash → skip.
    if has_completed(state, &activity.id)? {
        return Ok(ActivityResult::default());
    }
    let result = match runner.run(activity) {
        Ok(r) => r,
        Err(e) => {
            state.append(build_event(
                state,
                actor,
                EventType::ActivityFailed,
                activity,
                json!({ "error": e.to_string() }),
            ))?;
            return Err(e);
        }
    };
    // Checkpoint: append completion to the log, then let the broadcast wake
    // the PM loop (no direct pm.evaluate — see module doc).
    state.append(build_event(
        state,
        actor,
        EventType::ActivityCompleted,
        activity,
        json!({ "result_ref": result.result_ref }),
    ))?;
    Ok(result)
}

/// Boot-time recovery: re-dispatch every activity that was scheduled but never
/// completed/failed (i.e. the server died mid-activity). The idempotency guard
/// in [`execute`] makes the re-run safe. Returns the ids re-dispatched.
pub fn redispatch_inflight(
    state: &AppState,
    runner: &dyn ActivityRunner,
    actor: Actor,
) -> Result<Vec<String>> {
    let events = state.store.read_since(&state.project, 0)?;
    let mut dispatched = Vec::new();
    for e in events
        .iter()
        .filter(|e| e.event_type == EventType::ActivityScheduled)
    {
        let id = e.data.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() || is_terminated(state, id)? {
            continue;
        }
        let activity: Activity =
            serde_json::from_value(e.data.get("activity").cloned().unwrap_or(json!({})))
                .map_err(|_| anyhow!("cannot reconstruct ActivityScheduled {id}"))?;
        execute(state, runner, actor.clone(), &activity)?;
        dispatched.push(activity.id);
    }
    Ok(dispatched)
}

/// Build an activity event. The `Activity` is embedded as `data.activity`
/// (durable, reconstructable); per-type fields go in `extra`.
fn build_event(
    state: &AppState,
    actor: Actor,
    event_type: EventType,
    activity: &Activity,
    extra: serde_json::Value,
) -> Event {
    let mut data = serde_json::Map::new();
    data.insert("id".into(), json!(activity.id));
    data.insert("target_id".into(), json!(activity.target_id));
    data.insert(
        "activity".into(),
        serde_json::to_value(activity).unwrap_or(json!({})),
    );
    if let Some(obj) = extra.as_object() {
        for (k, v) in obj {
            data.insert(k.clone(), v.clone());
        }
    }
    Event::new(
        &state.project,
        actor,
        event_type,
        Aggregate {
            kind: "task".into(),
            id: activity.target_id.clone(),
        },
        serde_json::Value::Object(data),
    )
}
