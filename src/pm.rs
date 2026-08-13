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

use crate::actions::{self, PmAction, OWNER};
use crate::event::{Actor, Event, EventType};
use crate::policy::DecisionClass;
use crate::projection::Projection;
use crate::store::EventStore;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

pub const PM_CONSUMER: &str = "pm";

/// Stable agent roster the simulated company uses.
const AGENT_PM: &str = PM_CONSUMER;
const AGENT_ENG: &str = "marcus-reed";
const AGENT_QA: &str = "maya-patel";

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
    pub cursors: Arc<dyn crate::cursor::CursorStore>,
    pub project: String,
    /// Optional projection snapshot store (SEMANTIC_EVENTS §18). When present,
    /// projections are built from snapshot + tail (an optimization, never a
    /// source of truth); when None, the full log is folded. Optional so tests
    /// and simple runs need no snapshot store.
    pub snapshots: Option<Arc<dyn crate::snapshot::SnapshotStore>>,
    /// Pause inserted between appended events so the UI animates the company
    /// working. Zero in tests for speed. (brief §35)
    pub step_delay: Duration,
    /// Optional D2 orchestrator. When present, the PM routes new owner messages
    /// through it (instead of the scripted plan) — the LLM seam. **Off by
    /// default**: the real provider stays unplugged until the owner enables it.
    pub orchestrator: Option<Arc<dyn crate::orchestrator::Orchestrator>>,
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
    /// derived state (e.g. same-subject opinion contradictions). Cursor-gated,
    /// mirrors the PM loop. Set low in tests.
    pub reconcile_interval: u64,
    /// The workspace (set by `cast run`). Lets the PM physically provision
    /// worktrees (git worktree add) when a consultant is summoned. `None` in
    /// tests without a real repo.
    pub workspace: Option<Arc<crate::workspace::Workspace>>,
    events: Arc<broadcast::Sender<Event>>,
}

impl AppState {
    pub fn new<S, C>(store: S, cursors: C, project: impl Into<String>) -> Self
    where
        S: crate::store::EventStore + 'static,
        C: crate::cursor::CursorStore + 'static,
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
            workspace: None,
            events: Arc::new(tx),
        }
    }

    /// Builder-style: set how often (per appended event) the drift reconciler
    /// runs. Low in tests; tuned at runtime.
    pub fn with_reconcile_interval(mut self, n: u64) -> Self {
        self.reconcile_interval = n;
        self
    }

    /// Builder-style: attach the workspace so the PM can physically provision
    /// isolated worktrees when a consultant is summoned.
    pub fn with_workspace(mut self, workspace: Arc<crate::workspace::Workspace>) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Builder-style: enable projection snapshots for this AppState.
    pub fn with_snapshots<T: crate::snapshot::SnapshotStore + 'static>(
        mut self,
        snapshots: T,
    ) -> Self {
        self.snapshots = Some(Arc::new(snapshots));
        self
    }

    /// Builder-style: enable the D2 orchestrator (the LLM seam). Off by default.
    pub fn with_orchestrator(
        mut self,
        orchestrator: Arc<dyn crate::orchestrator::Orchestrator>,
    ) -> Self {
        self.orchestrator = Some(orchestrator);
        self
    }

    /// Builder-style: enforce write-time stream integrity on append.
    pub fn with_integrity(mut self) -> Self {
        self.enforce_integrity = true;
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

    /// Build the current projection, using the snapshot store when present.
    /// Reads come from snapshot + tail (or full fold); we also (re)store a
    /// snapshot as a side effect so the read path stays warm — but the event
    /// log remains the only authority.
    pub fn projection(&self) -> anyhow::Result<Projection> {
        match &self.snapshots {
            Some(snaps) => {
                let proj = crate::snapshot::build_from(&self.store, snaps, &self.project)?;
                let seq = self.store.latest_sequence(&self.project)?;
                let _ = snaps.save(&self.project, seq, &proj);
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
            crate::integrity::check_append(&proj, &event)?;
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
pub async fn run_pm(state: AppState, ws: crate::workspace::Workspace) {
    let mut rx = state.subscribe();
    loop {
        // Wake on any broadcast (cheap); a 500ms timeout is a safety poll so we
        // still catch events if broadcast lagged. Never per-event reasoning.
        let _ = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        // Observe the repo first — new commits become events the PM can react to.
        crate::git_observer::observe_once(&state, &ws).await;
        if let Err(e) = drain(&state).await {
            eprintln!("[pm] drain error: {e:#}");
        }
        // Drift reconciliation: every N events, clean up derived state.
        if let Err(e) = crate::reconciler::run_if_due(&state).await {
            eprintln!("[pm] reconciler error: {e:#}");
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
            Option<crate::orchestrator::CostMetering>,
        ) = if is_owner_message {
            // D2 seam: if an orchestrator is enabled, let IT drive the
            // response (the LLM, or the mock in tests). Otherwise use the
            // scripted plans.
            if let Some(orch) = &state.orchestrator {
                let context = projection.context_for("pm");
                let out = orch.plan(&context, e);
                (out.actions, out.metering)
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
                    "prompt_tokens": m.prompt_tokens,
                    "completion_tokens": m.completion_tokens,
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
async fn run_planned(state: &AppState, cause: &Event, planned: Vec<PlannedAction>) -> Result<u32> {
    if planned.is_empty() {
        return Ok(0);
    }
    let correlation = format!("run-{}", cause.sequence);
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
                eprintln!("[pm] policy gate rejected {who} action: {e}");
                rejected += 1;
                continue;
            }
        }
        for event in action.to_events(&state.project, &who, cause, &correlation) {
            // Structural isolation side-effect: when a WorktreeProvisioned
            // event is about to land, physically create the worktree via the
            // workspace (if one is attached). The event records it; the git op
            // makes it real. Idempotent at the Workspace level.
            if event.event_type == crate::event::EventType::WorktreeProvisioned {
                if let Some(ws) = &state.workspace {
                    let task_id = event
                        .data
                        .get("task_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let slug = "";
                    let port = event.data.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
                    if let Err(e) = ws.provision_worktree(&task_id, slug, port) {
                        eprintln!("[pm] worktree provision failed: {e:#}");
                    }
                }
            }
            if event.event_type == crate::event::EventType::CommitRequested {
                // Agent git surface side-effect: perform the commit physically
                // in the task's worktree (the intent is recorded as the event;
                // the git runner makes it real; the observer then records the
                // CommitObserved). No workspace attached (tests) → the intent
                // is still recorded, no physical commit.
                if let Some(ws) = &state.workspace {
                    let task_id = event.aggregate.id.clone();
                    let message = event
                        .data
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("checkpoint");
                    if let Err(e) = ws.commit_in_worktree(&task_id, message) {
                        eprintln!("[pm] worktree commit failed: {e:#}");
                    }
                }
            }
            state.append(event.clone())?;
            projection.apply(&event);
            authored += 1;
            if !state.step_delay.is_zero() {
                tokio::time::sleep(state.step_delay).await;
            }
        }
    }

    if rejected > 0 {
        eprintln!("[pm] rejected {rejected} invalid action(s) this pass");
    }
    Ok(authored)
}

/// Build a `ProvisionWorktree` action for a task, allocating a distinct port
/// from the pool (the lowest free one not already used by a provisioned
/// worktree OR allocated earlier in the same plan — plans can provision several
/// worktrees before any event executes, so we must exclude already-claimed
/// ports of this plan too). Isolated workspaces are the platform's structural
/// guarantee (2026-08-12) — the action the PM plans so a consultant is handed
/// a ready desk, never asked to "remember" to isolate.
fn plan_worktree_provision(
    state: &AppState,
    task_id: &str,
    slug: &str,
    claimed_in_plan: &mut std::collections::HashSet<u16>,
) -> PmAction {
    let projection = state
        .projection()
        .unwrap_or_else(|_| crate::projection::Projection::default());
    let used_in_projection: std::collections::HashSet<u16> =
        projection.worktrees.iter().map(|w| w.port).collect();
    let base = crate::port::worktree_base_port();
    let span = crate::port::WORKTREE_PORT_POOL;
    let port = (base..base.saturating_add(span))
        .find(|p| !used_in_projection.contains(p) && !claimed_in_plan.contains(p))
        .unwrap_or(crate::port::DEFAULT_WORKTREE_BASE_PORT);
    claimed_in_plan.insert(port);
    let cargo_target_dir = match &state.workspace {
        Some(ws) => ws
            .worktree_path(task_id)
            .join("target")
            .to_string_lossy()
            .into_owned(),
        None => format!(".casting/worktrees/{task_id}/target"),
    };
    PmAction::ProvisionWorktree {
        task_id: task_id.to_string(),
        slug: slug.to_string(),
        cargo_target_dir,
        port,
    }
}

/// First owner message: onboard the company and kick off a build. Plans the
/// whole sequence as actions; the gate lets each through as the projection
/// grows.
fn plan_onboard(
    state: &AppState,
    cause: &Event,
    body: &str,
    policy: &crate::policy::DecisionPolicy,
) -> Vec<PlannedAction> {
    let title = if body.trim().is_empty() {
        "the product".to_string()
    } else {
        body.trim().to_string()
    };

    // The testing-library decision is auto-decided by the PM ONLY when the
    // (event-sourced) policy routes it to the agent. If the owner has
    // escalated it to Ask, the PM proposes it and leaves it in the owner's
    // inbox — no auto-decision, no follow-up task.
    let testing_lib_decider = policy.resolve(DecisionClass::TestingLibrary).decider();

    // Onboard the working team: hire the default cast members by role, but SKIP
    // anyone the setup engine already hired. If setup explicitly chose a cast
    // (any non-PM agent exists), we DON'T top-up — the owner's chosen team
    // stands. The PM is hired separately at seed, so the cast here is the
    // working team.
    let already_hired: Vec<String> = state
        .projection()
        .ok()
        .map(|p| p.agents.iter().map(|a| a.id.clone()).collect())
        .unwrap_or_default();
    let has_existing_cast = already_hired.iter().any(|id| id != "pm"); // any non-PM agent = setup chose the cast
    let cast_hires: Vec<PlannedAction> = if has_existing_cast {
        Vec::new()
    } else {
        crate::cast::DEFAULT_CAST
            .iter()
            .filter(|m| !already_hired.iter().any(|id| id == m.agent_id))
            .map(|m| {
                let role = crate::cast::role_by_id(m.role_id).unwrap_or_else(|| {
                    panic!("default cast role {} missing from catalog", m.role_id)
                });
                (
                    "system".into(),
                    PmAction::HireAgent {
                        agent_id: m.agent_id.into(),
                        role: role.title.into(),
                    },
                )
            })
            .collect()
    };

    let mut plan: Vec<PlannedAction> = vec![
        (
            AGENT_PM.into(),
            PmAction::CreateRequirement {
                id: format!("req-{}", cause.event_id),
                title: title.clone(),
                description: body.to_string(),
            },
        ),
        (
            AGENT_PM.into(),
            PmAction::SendMessage {
                to: "owner".into(),
                body: format!("Understood — \u{201c}{title}\u{201d}. I've broken this into tasks and brought in Marcus (engineering) and Maya (QA). Stand by."),
            },
        ),
        (
            AGENT_PM.into(),
            PmAction::CreateTask {
                id: "task-design".into(),
                title: format!("Design {title}"),
                kind: "feature".into(),
            },
        ),
        (
            AGENT_PM.into(),
            PmAction::AssignTask { task_id: "task-design".into(), assignee: AGENT_ENG.into() },
        ),
        (
            AGENT_ENG.into(),
            PmAction::StartTask { task_id: "task-design".into() },
        ),
        (
            AGENT_ENG.into(),
            PmAction::CompleteTask { task_id: "task-design".into(), result: format!("Designed {title}") },
        ),
        (
            AGENT_PM.into(),
            PmAction::CreateTask {
                id: "task-core".into(),
                title: format!("Implement {title} core"),
                kind: "feature".into(),
            },
        ),
        (
            AGENT_PM.into(),
            PmAction::AssignTask { task_id: "task-core".into(), assignee: AGENT_ENG.into() },
        ),
        (AGENT_ENG.into(), PmAction::StartTask { task_id: "task-core".into() }),
        (
            AGENT_QA.into(),
            PmAction::CreateObservation {
                id: "obs-1".into(),
                severity: "info".into(),
                subject: "HTTPS not enabled in the scaffold".into(),
                body: "Noted during review. Won't fix now, but worth a task later.".into(),
                pm_action_required: false,
            },
        ),
        (
            AGENT_ENG.into(),
            PmAction::CompleteTask {
                task_id: "task-core".into(),
                result: format!("Core implementation of {title} done"),
            },
        ),
        // Marcus submits the core work for review; the PM routes it to QA.
        (
            AGENT_ENG.into(),
            PmAction::RequestReview {
                task_id: "task-core".into(),
                reviewer: AGENT_QA.into(),
            },
        ),
        (
            AGENT_QA.into(),
            PmAction::ReviewTask {
                task_id: "task-core".into(),
                approved: true,
                note: Some("Core looks solid — marcus integrates and ships".into()),
            },
        ),
        (
            AGENT_PM.into(),
            PmAction::CreateTask {
                id: "task-qa".into(),
                title: "Set up automated tests".into(),
                kind: "feature".into(),
            },
        ),
        (
            AGENT_PM.into(),
            PmAction::AssignTask { task_id: "task-qa".into(), assignee: AGENT_QA.into() },
        ),
        (AGENT_QA.into(), PmAction::StartTask { task_id: "task-qa".into() }),
        (
            AGENT_QA.into(),
            PmAction::CompleteTask { task_id: "task-qa".into(), result: "Test suite passing".into() },
        ),
        (
            AGENT_PM.into(),
            PmAction::ProposeDecision {
                id: "decision-db".into(),
                subject: "Database choice".into(),
                options: serde_json::json!({
                    "A": "PostgreSQL — robust, more infra, approx $18",
                    "B": "SQLite — dead simple, zero infra, approx $9"
                }),
                recommendation: "A".into(),
                // Resolve the involvement from the configured (event-sourced)
                // policy; Database defaults to Ask -> routes to the OWNER.
                class: DecisionClass::Database,
                involvement: policy.resolve(DecisionClass::Database),
            },
        ),
        (
            AGENT_PM.into(),
            PmAction::SendMessage {
                to: "owner".into(),
                body: "We need one call from you: which database for this build? I recommend A (PostgreSQL) for headroom, but B (SQLite) is zero-infra and cheaper.".into(),
            },
        ),
        // Delegated authority demo: choosing the testing library is a Pm-class
        // decision, so the PM decides it itself — DecisionProposed then
        // DecisionMade (actor = PM), no owner question, but fully recorded.
        (
            AGENT_PM.into(),
            PmAction::ProposeDecision {
                id: "decision-testing-lib".into(),
                subject: "Automated-testing library".into(),
                options: serde_json::json!({
                    "A": "pytest — batteries included",
                    "B": "cargo test — keep it in Rust"
                }),
                recommendation: "B".into(),
                class: DecisionClass::TestingLibrary,
                involvement: policy.resolve(DecisionClass::TestingLibrary),
            },
        ),
    ];
    // Hire the default cast first, before any work is planned.
    plan.splice(0..0, cast_hires);

    // Auto-decide the testing-library decision ONLY when the policy routes it
    // to the agent. If the owner escalated it to Ask, leave it open in their
    // inbox (Proposed) with no follow-up until they rule.
    if testing_lib_decider == crate::policy::Decider::Agent {
        plan.push((
            AGENT_PM.into(),
            PmAction::MakeDecision {
                decision_id: "decision-testing-lib".into(),
                approved: true,
                note: Some("PM: choosing cargo test, keep the toolchain single-language".into()),
            },
        ));
        plan.push((
            AGENT_PM.into(),
            PmAction::CreateTask {
                id: "task-testing-lib".into(),
                title: "Set up testing library (cargo test)".into(),
                kind: "feature".into(),
            },
        ));
    }

    let mut claimed_ports: std::collections::HashSet<u16> = std::collections::HashSet::new();

    // Structural isolation (2026-08-12): before any consultant STARTS a task,
    // the platform provisions its isolated worktree (own branch/build-target/
    // port). Walk the plan and insert ProvisionWorktree ahead of each StartTask
    // for a task assigned to a hired agent (not the owner). The gate already
    // rejects StartTask without a worktree, so this is what makes onboarding
    // actually work.
    let mut i = 0;
    while i < plan.len() {
        if let (_, PmAction::StartTask { task_id }) = &plan[i] {
            let assigned_to_consultant = plan.iter().any(|(_, a)| {
                matches!(a, PmAction::AssignTask { task_id: tid, assignee }
                    if tid == task_id && assignee != OWNER)
            });
            if assigned_to_consultant {
                let prov = (
                    AGENT_PM.into(),
                    plan_worktree_provision(state, task_id, "", &mut claimed_ports),
                );
                plan.insert(i, prov);
                i += 1; // skip the just-inserted provision
            }
        }
        i += 1;
    }

    plan
}

/// Owner just messaged but we already have requirements — acknowledge politely.
fn plan_acknowledge(cause: &Event) -> Vec<PlannedAction> {
    let body = cause
        .data
        .get("body")
        .and_then(|b| b.as_str())
        .unwrap_or("");
    vec![(
        AGENT_PM.into(),
        PmAction::SendMessage {
            to: "owner".into(),
            body: format!("Noted: \u{201c}{body}\u{201d}. It's on the backlog — I'll fold it into the next build pass."),
        },
    )]
}

/// The owner ruled on a proposed decision — plan the verdict's consequences.
fn plan_owner_decision(state: &AppState, cause: &Event) -> Vec<PlannedAction> {
    let decision_id = cause.aggregate.id.clone();
    let approved = cause
        .data
        .get("approved")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let note = cause
        .data
        .get("note")
        .and_then(|b| b.as_str())
        .unwrap_or("");
    let subject = cause
        .data
        .get("subject")
        .and_then(|b| b.as_str())
        .unwrap_or("your decision");

    let mut out = vec![(
        AGENT_PM.into(),
        PmAction::SendMessage {
            to: "owner".into(),
            body: if approved {
                format!("Great — \u{201c}{subject}\u{201d} approved{}. I'll drive the implementation now.", fmt_note(note))
            } else {
                format!("Understood — \u{201c}{subject}\u{201d} was declined{}. Discarding that option.", fmt_note(note))
            },
        },
    )];

    if approved {
        // If this decision was a GovernanceChange (PM proposed a directive
        // change), applying it is the OWNER's prerogative: only the owner may
        // author directives. The owner just approved it via DecisionMade, so we
        // author the directive change AS the owner — the approval is authority.
        let governance = ApprovedGovernanceChange::from_decision(state, &decision_id);
        if let Some(gov) = governance {
            out.push((
                "owner".into(),
                PmAction::CreateDirective {
                    id: gov.directive_id.clone(),
                    kind: gov.kind,
                    statement: gov.statement,
                    scope: gov.scope,
                    strength: gov.strength,
                    supersedes: gov.supersedes.clone(),
                },
            ));
            if let Some(superseded) = gov.supersedes {
                out.push((
                    "owner".into(),
                    PmAction::SupersedeDirective {
                        directive_id: superseded,
                        by_directive_id: gov.directive_id.clone(),
                    },
                ));
            }
        }

        // A PM-proposed consultant hire (AddConsultant class) is applied on
        // owner approval: the owner said yes, so the hire proceeds.
        let consultant = approved_consultant_role(state, &decision_id);
        if let Some(role_id) = consultant {
            out.push((
                "system".into(),
                PmAction::HireAgent {
                    agent_id: format!("{role_id}-1"),
                    role: crate::cast::role_by_id(&role_id)
                        .map(|r| r.title.to_string())
                        .unwrap_or_else(|| role_id.clone()),
                },
            ));
        }

        out.push((
            AGENT_PM.into(),
            PmAction::CreateTask {
                id: format!("task-adopt-{decision_id}"),
                title: format!("Adopt {subject} (owner-approved)"),
                kind: "feature".into(),
            },
        ));
        out.push((
            AGENT_PM.into(),
            PmAction::AssignTask {
                task_id: format!("task-adopt-{decision_id}"),
                assignee: AGENT_ENG.into(),
            },
        ));
        out.push((
            AGENT_ENG.into(),
            PmAction::StartTask {
                task_id: format!("task-adopt-{decision_id}"),
            },
        ));
        out.push((
            AGENT_ENG.into(),
            PmAction::CompleteTask {
                task_id: format!("task-adopt-{decision_id}"),
                result: format!("Adopted {subject}"),
            },
        ));
    }

    out
}

/// A GovernanceChange decision that the owner approved: the directive change to
/// apply, authored as the owner. Parsed from the DecisionProposed's `options`.
struct ApprovedGovernanceChange {
    directive_id: String,
    kind: crate::directive::DirectiveKind,
    statement: String,
    scope: Vec<String>,
    strength: crate::directive::DirectiveStrength,
    supersedes: Option<String>,
}

impl ApprovedGovernanceChange {
    /// Rebuild the projection, find the decision, and if it's an approved
    /// GovernanceChange, extract the proposed directive change.
    fn from_decision(state: &AppState, decision_id: &str) -> Option<Self> {
        // Use AppState::projection() (snapshot-aware) — the single projection
        // entry point. Never rebuild directly from the store.
        let proj = state.projection().ok()?;
        let dec = proj.decisions.iter().find(|d| d.id == decision_id)?;
        if dec.class != crate::policy::DecisionClass::GovernanceChange {
            return None;
        }
        let change = dec.options.get("governance_change")?;
        let kind: crate::directive::DirectiveKind =
            serde_json::from_value(change.get("kind")?.clone()).ok()?;
        let strength: crate::directive::DirectiveStrength =
            serde_json::from_value(change.get("strength")?.clone()).ok()?;
        let scope: Vec<String> = serde_json::from_value(change.get("scope")?.clone()).ok()?;
        Some(ApprovedGovernanceChange {
            directive_id: format!("directive-{decision_id}"),
            kind,
            statement: change.get("statement")?.as_str()?.to_string(),
            scope,
            strength,
            supersedes: change
                .get("supersedes")
                .and_then(|v| v.as_str())
                .map(String::from),
        })
    }
}

fn fmt_note(note: &str) -> String {
    if note.is_empty() {
        String::new()
    } else {
        format!(" (\u{201c}{note}\u{201d})")
    }
}

/// An AddConsultant decision that the owner approved: the role to hire.
/// Parsed from the DecisionProposed's `options`.
fn approved_consultant_role(state: &AppState, decision_id: &str) -> Option<String> {
    // Single projection entry point (snapshot-aware).
    let proj = state.projection().ok()?;
    let dec = proj.decisions.iter().find(|d| d.id == decision_id)?;
    if dec.class != crate::policy::DecisionClass::AddConsultant {
        return None;
    }
    dec.options
        .get("consultant")?
        .get("role_id")?
        .as_str()
        .map(str::to_string)
}
