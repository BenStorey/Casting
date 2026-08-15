//! Simulated Project Manager — the vertical slice's "company" control loop.
//!
//! Per docs/ADDENDUM.md the PM is a control loop over the event stream, not a
//! chatbot (§1), holds a durable cursor (§2), and turns owner input into
//! organizational work. This slice uses a deterministic *scripted* PM (D2:
//! scripted-first) that reacts to owner messages/decisions by producing the
//! SAME typed `PmAction`s an LLM will later emit (docs/ADDENDUM.md §16), which
//! are then passed through the policy gate in `actions.rs` before they become
//! domain events. That gate is the seam: swap the scripted planner below for a
//! real provider client later and the loop stays identical.
//!
//! Wake vs act: the loop wakes on a cheap notification (a broadcast of newly
//! appended events) and drains EVERYTHING since its cursor in one pass, then
//! advances the cursor — it never reasons per-event (docs/PM_INVOCATION_TRIGGERS.md).

use crate::actions::{self, PmAction};
use crate::event::{Actor, Event, EventType};
use crate::pm::planning::{plan_acknowledge, plan_onboard, plan_owner_decision};
use crate::projection::Projection;
use crate::store::EventStore;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

pub const PM_CONSUMER: &str = "pm";

/// Max events a snapshot may lag before the read path catches it up (writes).
/// Below this, `projection()` folds the tail in-memory and skips the write so a
/// busy `/api/state` isn't a snapshot write on every call.
const SNAPSHOT_CATCHUP: i64 = 64;

/// A proposed action plus the actor performing it. `who` is a label
/// (agent id, "owner", or "system") — converted to `Actor` at execution.
pub type PlannedAction = (String, PmAction);

/// Shared runtime state: the event store, durable cursors, the active project,
/// a broadcast channel for notifying subscribers (UI/SSE and the PM) that
/// events were appended, and the per-event animation delay (brief §35).
/// A notification is a *hint to consume persisted events*, never the source of
/// truth (brief §17).
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn crate::store::EventStore>,
    pub cursors: Arc<dyn crate::store::CursorStore>,
    pub project: String,
    /// Optional projection snapshot store (SEMANTIC_EVENTS §18). When present,
    /// projections are built from snapshot + tail (an optimization, never a
    /// source of truth); when None, the full log is folded. Optional so tests
    /// and simple runs need no snapshot store.
    pub snapshots: Option<Arc<dyn crate::store::SnapshotStore>>,
    /// Pause inserted between appended events so the UI animates the company
    /// working. Zero in tests for speed. (brief §35)
    pub step_delay: Duration,
    /// Optional D2 orchestrator. When present, the PM routes new owner messages
    /// through it (instead of the scripted plan) — the LLM seam. **Off by
    /// default**: the real provider stays unplugged until the owner enables it.
    pub orchestrator: Option<Arc<dyn crate::runtime::orchestrator::Orchestrator>>,
    /// When true, `append` enforces write-time stream integrity (events can't
    /// be appended without their precondition). Opt-in so fixtures/tests that
    /// hand-append bare events keep working.
    pub enforce_integrity: bool,
    /// Owner bearer token guarding the owner-mutating API endpoints. `None` =
    /// auth disabled (backward compatible with tests / local runs). Enabled via
    /// `with_owner_auth` / the `CAST_OWNER_TOKEN` env var.
    pub auth_token: Option<Arc<str>>,
    /// The state dir (set by `cast run`). Lets the web setup endpoint persist
    /// `config.json` (name + owner token). `None` in tests.
    pub state_dir: Option<std::path::PathBuf>,
    /// Every N appended events, the drift reconciler wakes and cleans up
    /// derived state. Cursor-gated, mirrors the PM loop. Set low in tests.
    pub reconcile_interval: u64,
    /// The reconciliation passes registered on the cursor-gated cadence
    /// (2026-08-12, pluggable). Defaults to opinion-drift + stale-worktree
    /// prune; add new pass TYPES here without touching the loop.
    pub reconcile_passes: Vec<Arc<dyn crate::pm::reconciler::ReconcilePass>>,
    /// The workspace (set by `cast run`). Lets the PM physically provision
    /// worktrees (git worktree add) when a consultant is summoned. `None` in
    /// tests without a real repo.
    pub workspace: Option<Arc<crate::workspace::Workspace>>,
    /// When true, the PM's onboard plan promotes cross-cutting requirements to
    /// Feature Mode: decomposes them into parallel children and adds Blocker-Test
    /// hard edges (ordering). Opt-in so the canonical demo flow + tests stay
    /// flat by default; flip to default-on once the decomposed flow is proven.
    /// Enabled via `with_decompose` / the `CAST_DECOMPOSE` env var.
    pub decompose: bool,
    /// The per-project secret store (2026-08-13). `None` when unset (local
    /// runs / tests). When present, the executor refuses to schedule/execute an
    /// activity that embeds a raw secret value (the no-secret-in-log invariant).
    pub secrets: Option<Arc<crate::workspace::secrets::SecretStore>>,
    /// The loaded consultant registry (2026-08-13): the curated embedded
    /// defaults overlaid by any user packages in `<project>/.casting/consultants/`.
    /// Answers "what consultants exist + what are they configured to do" for the
    /// D2 orchestrator / `/api/consultants`. Configuration, never authority.
    pub consultants: Arc<crate::consultants::ConsultantRegistry>,
    /// The owner-facing external channel (2026-08-14): a best-effort transport
    /// for owner messaging (Telegram reference adapter). `NoopChannel` by
    /// default — a pipe to nowhere, off until configured. Never authoritative;
    /// the event log / projection stay the only truth.
    pub channel: Arc<dyn crate::runtime::channel::OwnerChannel>,
    /// Guards against double-spawning the Telegram run loop (2026-08-14): it
    /// can be started from boot env OR from the UI `POST /api/telegram/configure`,
    /// but must run exactly once. Set the first time a channel is attached.
    pub telegram_started: Arc<std::sync::atomic::AtomicBool>,
    /// The running Telegram loop's JoinHandle (2026-08-14, batch 3): lets a
    /// reconfigure (new bot token / chat) abort the old loop and start a fresh
    /// one — so messaging can be reconnected any time, not just at boot.
    pub telegram_handle: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    events: Arc<broadcast::Sender<Event>>,
}

impl AppState {
    pub fn new<S, C>(store: S, cursors: C, project: impl Into<String>) -> Self
    where
        S: crate::store::EventStore + 'static,
        C: crate::store::CursorStore + 'static,
    {
        let (tx, _) = broadcast::channel(1024);
        AppState {
            store: Arc::new(store),
            cursors: Arc::new(cursors),
            project: project.into(),
            snapshots: None,
            step_delay: Duration::from_millis(220),
            orchestrator: None,
            enforce_integrity: false,
            auth_token: None,
            state_dir: None,
            reconcile_interval: 25,
            reconcile_passes: crate::pm::reconciler::default_passes(),
            workspace: None,
            decompose: false,
            secrets: None,
            // Curated defaults are embedded and always load; a malformed default
            // would be a bug, so falling back to an empty registry is the safe
            // degradation (the real defaults are validated by their own test).
            consultants: Arc::new(
                crate::consultants::ConsultantRegistry::from_embedded().unwrap_or_default(),
            ),
            channel: Arc::new(crate::runtime::channel::NoopChannel),
            telegram_started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            telegram_handle: Arc::new(std::sync::Mutex::new(None)),
            events: Arc::new(tx),
        }
    }

    /// Builder-style: set how often (per appended event) the drift reconciler
    /// runs. Low in tests; tuned at runtime.
    pub fn with_reconcile_interval(mut self, n: u64) -> Self {
        self.reconcile_interval = n;
        self
    }

    /// Builder-style: add a reconciliation pass (2026-08-12). Reconciliation is
    /// pluggable — append new pass types without touching the loop.
    pub fn with_reconcile_pass(
        mut self,
        pass: Arc<dyn crate::pm::reconciler::ReconcilePass>,
    ) -> Self {
        self.reconcile_passes.push(pass);
        self
    }

    /// Builder-style: attach the workspace so the PM can physically provision
    /// isolated worktrees when a consultant is summoned.
    pub fn with_workspace(mut self, workspace: Arc<crate::workspace::Workspace>) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Builder-style: enable projection snapshots for this AppState.
    pub fn with_snapshots<T: crate::store::SnapshotStore + 'static>(
        mut self,
        snapshots: T,
    ) -> Self {
        self.snapshots = Some(Arc::new(snapshots));
        self
    }

    /// Builder-style: attach the owner-facing external channel (Telegram
    /// reference adapter). `NoopChannel` by default.
    pub fn with_channel(mut self, channel: Arc<dyn crate::runtime::channel::OwnerChannel>) -> Self {
        self.channel = channel;
        self
    }

    /// Builder-style: enable the D2 orchestrator (the LLM seam). Off by default.
    pub fn with_orchestrator(
        mut self,
        orchestrator: Arc<dyn crate::runtime::orchestrator::Orchestrator>,
    ) -> Self {
        self.orchestrator = Some(orchestrator);
        self
    }

    /// Builder-style: enforce write-time stream integrity on append.
    pub fn with_integrity(mut self) -> Self {
        self.enforce_integrity = true;
        self
    }

    /// Builder-style: enable the PM's automatic Feature-Mode decomposition
    /// (cross-cutting requirements fan out into parallel ordered children).
    pub fn with_decompose(mut self) -> Self {
        self.decompose = true;
        self
    }

    /// Builder-style: attach the per-project secret store (2026-08-13). The
    /// executor then refuses to schedule/execute an activity that embeds a raw
    /// secret value (the no-secret-in-log invariant).
    pub fn with_secrets(mut self, secrets: crate::workspace::secrets::SecretStore) -> Self {
        self.secrets = Some(Arc::new(secrets));
        self
    }

    /// Builder-style: replace the consultant registry (e.g. the curated
    /// defaults overlaid with user packages from `.casting/consultants/`).
    pub fn with_consultants(
        mut self,
        consultants: Arc<crate::consultants::ConsultantRegistry>,
    ) -> Self {
        self.consultants = consultants;
        self
    }

    /// Builder-style: enable owner auth with a bearer token. The owner-mutating
    /// API endpoints then require `Authorization: Bearer <token>`.
    pub fn with_owner_auth(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(Arc::from(token.into()));
        self
    }

    /// Builder-style: attach the state dir (used by the web setup endpoint).
    pub fn with_state_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.state_dir = Some(dir.into());
        self
    }

    /// D2 LLM wiring helper (used by `cast run`): when the environment carries
    /// an LLM API key, build the real OpenAI-compatible orchestrator from
    /// `ProviderConfig::from_env()` and attach it. Unconfigured → the state is
    /// returned unchanged (deterministic scripted PM stays the default, no
    /// spend). The consultant registry's default system prompt seeds the LLM's
    /// persona when one is available.
    pub fn pipe_llm_orchestrator(self) -> Self {
        let state_dir = self.state_dir.as_deref();
        match crate::llm::config::from_env(state_dir) {
            Ok(Some(cfg)) => {
                // The PM persona: prefer a consultant bound to the pm role if one
                // exists, else the canonical PM identity (the registry holds the
                // agent cast — Marcus/Maya/etc. — so don't borrow an engineer's
                // persona for the Project Manager).
                let persona = self
                    .consultants
                    .for_role("pm")
                    .and_then(|c| c.system_prompt.clone())
                    .unwrap_or_else(|| {
                        "You are Sarah Chen, the Project Manager. You organize a team of \
                         specialist consultants to turn the owner's intent into a working \
                         plan."
                            .to_string()
                    });
                println!(
                    "🧠 LLM orchestrator enabled: provider={} model={} base={}",
                    cfg.provider, cfg.model, cfg.base_url
                );
                // Per-actor routing: the env config is the base; consultants
                // with a declared model binding route to their own model, key
                // falling back to env.
                let resolver = crate::llm::routing::ModelResolver::new(
                    cfg.clone(),
                    (*self.consultants).clone(),
                )
                .with_default_persona(persona.clone());
                self.with_orchestrator(Arc::new(
                    crate::llm::LlmOrchestrator::new(cfg, persona).with_resolver(resolver),
                ))
            }
            Ok(None) => self,
            Err(e) => {
                // Misconfiguration (e.g. a key but no model): warn loudly but
                // keep the deterministic PM rather than failing a run.
                eprintln!("⚠️  LLM misconfigured, using scripted PM: {e:#}");
                self
            }
        }
    }

    /// Build the current projection, using the snapshot store when present.
    /// Reads come from snapshot + tail (or full fold); we also (re)store a
    /// snapshot so the read path stays warm — but the event log remains the
    /// only authority. The save is THROTTLED: we only catch the snapshot up
    /// when it is stale by more than [`SNAPSHOT_CATCHUP`] events, so a live
    /// `/api/state` / background pass isn't a write on every call (esp. the
    /// Postgres backend, where writes serialize behind the single thread).
    pub fn projection(&self) -> anyhow::Result<Projection> {
        match &self.snapshots {
            Some(snaps) => {
                let proj = crate::store::build_from(&self.store, snaps, &self.project)?;
                let seq = self.store.latest_sequence(&self.project)?;
                let stale = snaps
                    .load(&self.project)
                    .map(|(last_seq, _)| seq - last_seq > SNAPSHOT_CATCHUP)
                    .unwrap_or(true);
                if stale {
                    let _ = snaps.save(&self.project, seq, &proj);
                }
                Ok(proj)
            }
            None => Projection::build(&self.store, &self.project),
        }
    }

    /// Builder-style setter used by tests to disable the animation pause.
    pub fn with_step_delay(mut self, delay: Duration) -> Self {
        self.step_delay = delay;
        self
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Broadcast an already-persisted event to subscribers (wake hint for the
    /// PM, realtime push for the UI/SSE). Used by the git observer after it
    /// appends events directly to the store (bypassing `append`).
    pub fn notify(&self, event: &Event) {
        let _ = self.events.send(event.clone());
    }

    /// Append an event to the store, assign its sequence, then broadcast it to
    /// subscribers (a wake hint for the PM, a realtime push for the UI).
    pub fn append(&self, event: Event) -> Result<Event> {
        if self.enforce_integrity {
            let proj = Projection::build(&self.store, &self.project)?;
            crate::event::integrity::check_append(&proj, &event)?;
        }
        let stored = self.store.append(event)?;
        // Ignore send errors: nobody listening just means no one cares yet.
        let _ = self.events.send(stored.clone());
        Ok(stored)
    }
}

/// Spawn the simulated PM loop. It blocks on wake hints, drains all new events
/// since its cursor, lets the scripted policy respond, then advances the cursor.
/// On each drain it also runs the git observer so new branches/commits become
/// semantic events before the PM reasons (Git slice increment 2).
///
/// WAKE ≠ ACT (docs/PM_INVOCATION_TRIGGERS.md, tiers in `crate::runtime::wake`): the
/// expensive ACT path (observe + drain + respond + reconciler) only runs when a
/// Tier-0/1 interrupt arrives OR the quiet window elapses. A lone Tier-2
/// (batch) event defers — the cursor keeps accumulating, and a later interrupt
/// or the poll timeout flushes it. This bounds LLM spend on progress churn.
pub async fn run_pm(state: AppState, ws: crate::workspace::Workspace) {
    let mut rx = state.subscribe();
    loop {
        // Wake on a broadcast (cheap); the 500ms timeout IS the quiet window.
        // Never per-event reasoning — a burst coalesces into one drain.
        let wake = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        // Decide whether to ACT: a non-batch (interrupt) event, or the quiet
        // window elapsed (a timeout in the receiver, or a lagged broadcast that
        // dropped events → flush the accumulated batch). A lone batch event
        // with no quiet window defers.
        let act = match wake {
            Ok(Ok(ev)) => {
                let tier = crate::runtime::wake::tier_of(ev.event_type);
                tier != crate::runtime::wake::WakeTier::Batch
            }
            // Timeout (nothing arrived in the window) or lagged-broadcast → the
            // quiet window elapsed; flush accumulated batch events.
            _ => true,
        };
        if act {
            // Observe the repo first — new commits become events the PM can react to.
            crate::workspace::git_observer::observe_once(&state, &ws).await;
            if let Err(e) = drain(&state).await {
                log::error!("[pm] drain error: {e:#}");
            }
            // Drift reconciliation: every N events, run every registered pass.
            if let Err(e) = crate::pm::reconciler::run_if_due(&state) {
                log::error!("[pm] reconciler error: {e:#}");
            }
        }
    }
}

/// Run one drain pass now (test/CLI entry). Consumes all events since the PM's
/// cursor, lets the scripted policy respond, and advances the cursor. Returns
/// how many events the PM authored. Idempotent: a second call with no new
/// events returns 0.
pub async fn drive_pm(state: &AppState) -> Result<u32> {
    drain(state).await
}

/// Drain everything since the PM's cursor in one pass and respond, then commit
/// the new cursor position. Returns the number of events the PM appended.
async fn drain(state: &AppState) -> Result<u32> {
    let cursor = state.cursors.get(&state.project, PM_CONSUMER)?;
    let new_events = state.store.read_since(&state.project, cursor.last_seen)?;
    if new_events.is_empty() {
        return Ok(0);
    }

    let projection = state.projection()?;
    let authored = respond(state, &projection, &new_events).await?;

    let latest = state.store.latest_sequence(&state.project)?;
    state.cursors.advance(&state.project, PM_CONSUMER, latest)?;
    Ok(authored)
}

/// The scripted policy: look at the events since the cursor and decide what the
/// PM "company" plans next. Deterministic for this slice. Returns the planned
/// actions, which `run_planned` validates and executes.
async fn respond(state: &AppState, projection: &Projection, new_events: &[Event]) -> Result<u32> {
    let mut authored = 0u32;

    for e in new_events {
        let (is_owner_message, body) = match e.event_type {
            EventType::MessageSent if e.actor == Actor::Owner => (
                true,
                e.data.get("body").and_then(|b| b.as_str()).unwrap_or(""),
            ),
            _ => (false, ""),
        };

        // D2/orchestrator path returns actions + optional cost metering; the
        // scripted paths return actions only. Normalize to (actions, metering).
        let (planned, metering): (
            Vec<PlannedAction>,
            Option<crate::runtime::orchestrator::CostMetering>,
        ) = if is_owner_message {
            // D2 seam: if an orchestrator is enabled, let IT drive the
            // response (the LLM, or the mock in tests). Otherwise use the
            // scripted plans.
            if let Some(orch) = &state.orchestrator {
                // Hard harness gate (2026-08-13, guard.rs): the circuit breaker
                // sits OUTSIDE the PM. If work is paused or the budget is
                // exhausted, do NOT issue the provider call (no spend) and skip
                // planning entirely.
                if let Err(reason) = crate::pm::guard::llm_dispatch_allowed(projection) {
                    log::warn!("[pm] guard blocked LLM dispatch: {reason}");
                    (Vec::new(), None)
                } else {
                    let context = projection.context_for("pm");
                    let correlation = format!("run-{}", e.sequence);
                    // The orchestrator is async + fallible now (a real provider
                    // call). On an LLM/provider error, record the failed pass in
                    // the diagnostics audit trail and produce no actions — no
                    // spend beyond the failed call, no panics. A flag keeps the
                    // error audit from being followed by an empty success audit
                    // (ONE OrchestrationRun per pass).
                    let mut planning_failed = false;
                    let out = match orch.plan(&context, e).await {
                        Ok(out) => out,
                        Err(err) => {
                            log::error!("[pm] orchestrator error: {err:#}");
                            planning_failed = true;
                            let _ = state.append(crate::pm::planning::orchestration_run_event(
                                &state.project,
                                &correlation,
                                serde_json::json!({
                                    "trigger": format!("{:?}", e.event_type),
                                    "actor": "pm",
                                    "correlation": correlation.clone(),
                                    "context_summary": crate::runtime::context::summary(&context),
                                    "error": format!("{err:#}"),
                                    "metered": false,
                                }),
                            ));
                            crate::runtime::orchestrator::PlanOutput::default()
                        }
                    };
                    // Audit the planning pass ONLY if it didn't already emit the
                    // error record above: what the model saw + decided.
                    if !planning_failed {
                        let planned_strs = out
                            .actions
                            .iter()
                            .map(|(who, a)| {
                                format!("{who} -> {}", serde_json::to_string(a).unwrap_or_default())
                            })
                            .collect::<Vec<_>>();
                        let m = out.metering.as_ref();
                        let _ = state.append(crate::pm::planning::orchestration_run_event(
                            &state.project,
                            &correlation,
                            serde_json::json!({
                                "trigger": format!("{:?}", e.event_type),
                                "actor": "pm",
                                "correlation": correlation.clone(),
                                "context_summary": crate::runtime::context::summary(&context),
                                "planned": planned_strs,
                                "metered": m.is_some(),
                                "metering_agent": m.map(|x| x.agent_id.clone()),
                                "provider": m.and_then(|x| x.provider.clone()),
                                "model": m.and_then(|x| x.model.clone()),
                                "prompt_tokens": m.map(|x| x.prompt_tokens).unwrap_or(0),
                                "completion_tokens": m.map(|x| x.completion_tokens).unwrap_or(0),
                                "latency_ms": m.map(|x| x.latency_ms).unwrap_or(0),
                                "estimated_usd": m.map(|x| x.estimated_usd).unwrap_or(0.0),
                            }),
                        ));
                    }
                    (out.actions, out.metering)
                }
            } else if projection.requirements.is_empty() {
                (plan_onboard(state, e, body, &projection.policy), None)
            } else {
                (plan_acknowledge(e), None)
            }
        } else if e.event_type == EventType::DecisionMade && e.actor == Actor::Owner {
            // Only an OWNER's decision needs a PM reaction. A PM-authored
            // DecisionMade (a delegated Pm/Never decision) was already handled
            // inline by the plan that made it — reacting again would duplicate.
            (plan_owner_decision(state, e), None)
        } else {
            (Vec::new(), None)
        };

        // HARNESS #6: land the provider cost in the event log so spend is
        // attributable per agent/task (the PM's budget concern reads it).
        if let Some(m) = metering {
            state.append(crate::event::Event::new(
                &state.project,
                Actor::System,
                EventType::CostIncurred,
                crate::event::Aggregate {
                    kind: "cost".into(),
                    id: uuid::Uuid::new_v4().to_string(),
                },
                serde_json::json!({
                    "agent_id": m.agent_id,
                    "task_id": m.task_id,
                    "model_tier": m.model_tier,
                    "model": m.model,
                    "provider": m.provider,
                    "prompt_tokens": m.prompt_tokens,
                    "completion_tokens": m.completion_tokens,
                    "cache_read_input_tokens": m.cache_read_input_tokens,
                    "cache_creation_input_tokens": m.cache_creation_input_tokens,
                    "latency_ms": m.latency_ms,
                    "input_price_per_mtok": m.input_price_per_mtok,
                    "output_price_per_mtok": m.output_price_per_mtok,
                    "estimated_usd": m.estimated_usd,
                }),
            ))?;
        }

        authored += run_planned(state, e, planned).await?;
    }

    Ok(authored)
}

/// Run planned actions through the policy gate, then execute the events that
/// pass. Each action is validated against a *running* projection (updated as we
/// append), so within one plan an earlier action's effect is visible to a later
/// validation. Returns how many events were appended.
///
/// IDEMPOTENT against re-entry: a mid-drain failure (a store append error that
/// propagates via `?` before the cursor advances) would otherwise make the next
/// drain re-read the SAME causes and re-plan the SAME actions, re-emitting
/// duplicate DOMAIN events. We therefore skip appending a real-entity domain
/// event that was ALREADY applied for this same planning cause (same
/// `event_type` + `aggregate.id` + correlation). Audit/telemetry records are
/// deliberately NEVER deduped — see [`dedup_applies`].
async fn run_planned(state: &AppState, cause: &Event, planned: Vec<PlannedAction>) -> Result<u32> {
    if planned.is_empty() {
        return Ok(0);
    }
    let correlation = format!("run-{}", cause.sequence);
    // One store read per plan: the set of real-entity domain events already in
    // the log (keyed by event_type + aggregate.id + correlation). Built once so
    // the whole pass dedups against the same picture of history. EventType has
    // no `Hash`, so the event_type is keyed by its Debug (variant) name.
    let mut applied: std::collections::HashSet<(String, String, String)> =
        applied_domain_keys(&state.store, &state.project)?;
    let mut projection = state.projection()?;
    let mut authored = 0u32;
    let mut rejected = 0u32;

    for (who, action) in planned {
        if action == PmAction::NoOp {
            continue;
        }
        match actions::validate(&action, &who, &projection) {
            Ok(()) => {}
            Err(e) => {
                log::warn!("[pm] policy gate rejected {who} action: {e}");
                // Audit the refusal in the event log so a misbehaving plan
                // (esp. the real LLM) is visible in the UI/stream, not just
                // stderr. Serialized PmAction + reason => exactly what was
                // attempted and why it was refused.
                let _ = state.append(crate::pm::planning::plan_rejected_event(
                    &state.project,
                    &correlation,
                    serde_json::json!({
                        "who": who,
                        "action": serde_json::to_string(&action).unwrap_or_default(),
                        "reason": e.to_string(),
                        "correlation": correlation.clone(),
                    }),
                ));
                rejected += 1;
                continue;
            }
        }
        for event in action.to_events(&state.project, &who, cause, &correlation) {
            // Idempotency guard: skip a real-entity DOMAIN event that was ALREADY
            // applied for this same planning cause. Without this, a mid-drain
            // failure (cursor not advanced) that re-drains the same causes would
            // re-emit a duplicate. Audit/telemetry records are NOT subject to
            // this (they keep appending as-is).
            if dedup_applies(&event) {
                let key = (
                    format!("{:?}", event.event_type),
                    event.aggregate.id.clone(),
                    correlation.clone(),
                );
                if applied.contains(&key) {
                    continue; // already emitted by a (failed) prior drain — skip, no re-count.
                }
                applied.insert(key);
            }
            // Durable intent FIRST. The domain event is the truth of what was
            // attempted; it must be in the log before we try to make it
            // physical, so a crash between intent and effect still has an
            // auditable record (and a re-drain can reconcile, not silently
            // re-fire against an unrecorded effect).
            state.append(event.clone())?;
            projection.apply(&event);
            authored += 1;

            // Event-driven workspace side effects (provision/commit) go through
            // the EXECUTOR seam — the same guarded path as any real side effect
            // (pause / budget / secret gates all apply), instead of inline hooks.
            // The WorkspaceRunner makes the already-appended intent physical. No
            // workspace attached (tests) → intent recorded, no physical op.
            if let Some(activity) = crate::runtime::executor::workspace_activity_for(&event) {
                if let Some(ws) = state.workspace.clone() {
                    let runner = crate::runtime::executor::WorkspaceRunner::new(ws);
                    if let Err(e) =
                        crate::runtime::executor::run_side_effect(state, &runner, &activity)
                    {
                        log::error!("[pm] workspace side-effect failed: {e:#}");
                        // Align the projection with physical reality: a
                        // WorktreeProvisioned whose physical `git worktree`
                        // never got created must not claim a desk (the fail-closed
                        // StartTask gate reads this). Reuse the WorktreeRemoved
                        // lifecycle close — it frees the port + removes the entry,
                        // and it is the opposite intent of the failed provision.
                        if event.event_type == crate::event::EventType::WorktreeProvisioned {
                            if let Some(task_id) =
                                event.data.get("task_id").and_then(|v| v.as_str())
                            {
                                let marker = crate::event::Event::new(
                                    &state.project,
                                    crate::event::Actor::System,
                                    crate::event::EventType::WorktreeRemoved,
                                    crate::event::Aggregate {
                                        kind: "worktree".into(),
                                        id: format!("worktree-{task_id}"),
                                    },
                                    serde_json::json!({
                                        "task_id": task_id,
                                        "cause": "provision-failed",
                                    }),
                                );
                                let _ = state.append(marker.clone());
                                projection.apply(&marker);
                            }
                        }
                    }
                }
            }
            // WRITE-TIME worktree teardown (2026-08-12, owner request): the
            // moment a task BECOMES Done (or its ChangeSet is merged), tear
            // down its worktree immediately — physical remove + WorktreeRemoved
            // event (frees the port). This is expected behavior ("cleanup as
            // soon as the agent is finished"), not something that should wait
            // for the periodic reconciler cadence.
            if matches!(
                event.event_type,
                crate::event::EventType::TaskCompleted
                    | crate::event::EventType::ChangeSetReady
                    | crate::event::EventType::MergeCompleted
            ) {
                if let Err(e) = crate::pm::reconciler::prune_worktrees(state) {
                    log::error!("[pm] write-time worktree prune failed: {e:#}");
                }
            }
            if !state.step_delay.is_zero() {
                tokio::time::sleep(state.step_delay).await;
            }
        }
    }

    if rejected > 0 {
        log::info!("[pm] rejected {rejected} invalid action(s) this pass");
    }
    Ok(authored)
}

/// Should the PM's idempotent-drain dedup apply to this event?
///
/// Dedup applies ONLY to real-entity DOMAIN events (task / decision /
/// requirement / opinion / risk / worktree / agent / observation / ...).
/// Audit / telemetry / guard records — `PlanActionRejected`, `OrchestrationRun`,
/// `CostIncurred`, `BudgetSet`, `WorkPaused`/`WorkResumed`, `MessageSent` acks,
/// and ANY aggregate kind `"plan"` — are NEVER deduped: they deliberately reuse
/// ONE shared aggregate id AND the `run-{seq}` correlation per planning pass,
/// so a naive dedup by (event_type, aggregate.id, correlation) would collapse
/// multiple distinct audit records and break the audit trail. They must keep
/// appending as-is.
fn dedup_applies(e: &Event) -> bool {
    use EventType::*;
    match e.event_type {
        PlanActionRejected | OrchestrationRun | CostIncurred | BudgetSet | WorkPaused
        | WorkResumed | MessageSent => false,
        // The "plan" aggregate is the shared audit aggregate — never dedup.
        _ => e.aggregate.kind != "plan",
    }
}

/// Collect the set of real-entity DOMAIN events already in the log, keyed by
/// `(event_type, aggregate.id, correlation_id)`. The PM drain uses this to skip
/// re-emitting an already-applied domain event for the same planning cause on
/// re-entry (a mid-drain failure must not duplicate events).
fn applied_domain_keys(
    store: &Arc<dyn crate::store::EventStore>,
    project: &str,
) -> Result<std::collections::HashSet<(String, String, String)>> {
    let mut keys = std::collections::HashSet::new();
    for e in store.read_since(project, 0)? {
        if dedup_applies(&e) {
            if let Some(corr) = &e.metadata.correlation_id {
                keys.insert((
                    format!("{:?}", e.event_type),
                    e.aggregate.id.clone(),
                    corr.clone(),
                ));
            }
        }
    }
    Ok(keys)
}
