//! The drift reconciler — a cursor-gated "every N events" cleanup pass.
//!
//! Knowledge accumulates and *drifts*: opinions don't go stale in a burst, they
//! slowly become inconsistent. Write-time supersession is eager and brittle
//! (the writer must know every target). Instead (owner framing 2026-08-10):
//! **keep writes simple; reconcile periodically.** This is a reusable primitive
//! — the same cursor-gated trigger drives MANY reconciliation types.
//!
//! It runs as its OWN consumer (mirrors the PM loop): a `RECONCILER_CONSUMER`
//! cursor + a threshold interval. When `latest - reconciler_cursor >= N`, it
//! wakes and runs every **registered pass** (`ReconcilePass`), then advances its
//! cursor. The "smart" judgment of *what* truly conflicts stays a D2 seam (the
//! LLM reviewer); the skeleton does the mechanically-obvious cleanup
//! deterministically.
//!
//! ## Many reconciliation types (owner, 2026-08-12)
//!
//! Reconciliation is pluggable: a `ReconcilePass` is any named, deterministic
//! cleanup that runs on the cadence. Passes are registered on `AppState`
//! (default: opinion-drift + stale-worktree prune). Adding a new type (e.g.
//! priority/plan re-ranking, stale-observation cleanup) is just a new pass — no
//! changes to the loop. Worktree teardown itself is also triggered at WRITE-TIME
//! (when a task completes), not only on the cadence; the periodic pass is a
//! safety net.

use crate::actions::PmAction;
use crate::pm::AppState;
use crate::projection::{OpinionStatus, Projection};
use anyhow::Result;
use std::sync::Arc;

/// The reconciler's durable position in the event stream.
pub const RECONCILER_CONSUMER: &str = "reconciler";

/// One named, deterministic reconciliation pass. A pass inspects the projection
/// and emits ordinary gate actions / events to clean up a specific class of
/// drift. Runs on the cursor-gated cadence (and passes may also be invoked at
/// write-time for eager cleanup, e.g. worktree teardown). Synchronous: a pass is
/// a bounded, mechanical cleanup — no awaits needed.
pub trait ReconcilePass: Send + Sync {
    /// A stable name for logging/diagnostics.
    fn name(&self) -> &'static str;
    /// Run this pass now. Returns how many events it appended.
    fn run(&self, state: &AppState) -> Result<u32>;
}

/// The default opinion-drift pass: supersede the older of two Active opinions
/// with the same subject (keeps the latest per subject).
pub struct OpinionDriftPass;

impl ReconcilePass for OpinionDriftPass {
    fn name(&self) -> &'static str {
        "opinion-drift"
    }
    fn run(&self, state: &AppState) -> Result<u32> {
        opinion_drift(state)
    }
}

/// The default stale-worktree pass: tear down worktrees whose task is Done or
/// whose ChangeSet is Merged (physical remove + `WorktreeRemoved` event, which
/// frees the port). A safety net on the cadence — eager teardown also happens at
/// write-time (see `pm::run_planned`).
pub struct StaleWorktreePass;

impl ReconcilePass for StaleWorktreePass {
    fn name(&self) -> &'static str {
        "stale-worktree"
    }
    fn run(&self, state: &AppState) -> Result<u32> {
        prune_worktrees(state)
    }
}

/// Archive terminal entities (done tasks, superseded decisions/opinions,
/// resolved risks) — remove them from the active projection and replace
/// with a compact summary. Saves agent context tokens.
pub struct ArchivePass;

impl ReconcilePass for ArchivePass {
    fn name(&self) -> &'static str {
        "archive-terminal"
    }
    fn run(&self, state: &AppState) -> Result<u32> {
        archive_terminals(state)
    }
}

/// Return the reconciler's registered passes (the defaults unless overridden).
pub fn default_passes() -> Vec<Arc<dyn ReconcilePass>> {
    vec![
        Arc::new(OpinionDriftPass),
        Arc::new(StaleWorktreePass),
        Arc::new(ArchivePass),
    ]
}

/// Whether the reconciler is due: if at least `interval` events have landed
/// since its last pass. Cursor-gated and idempotent — running it again with no
/// new events returns immediately.
pub fn should_run(state: &AppState) -> Result<bool> {
    if state.reconcile_interval == 0 {
        return Ok(false);
    }
    let cursor = state.cursors.get(&state.project, RECONCILER_CONSUMER)?;
    let latest = state.store.latest_sequence(&state.project)?;
    Ok(latest.saturating_sub(cursor.last_seen) >= state.reconcile_interval as i64)
}

/// One mechanically-detected drift to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    /// The older opinion (chronologically) — to be superseded.
    pub older_id: String,
    /// The newer Active opinion of the same subject that displaces it.
    pub by_id: String,
    pub subject: String,
}

/// Detect mechanical drift: two Active opinions with the SAME subject. The
/// projection folds events in chronological order, so within `projection.opinions`
/// the earlier entry is the older one. We keep the latest per subject and flag
/// the earlier duplicates. Empty subject = ungroupable (skipped).
pub fn drift(projection: &Projection) -> Vec<Drift> {
    let mut out: Vec<Drift> = Vec::new();
    let mut newest_by_subject: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for op in projection
        .opinions
        .iter()
        .filter(|o| o.status == OpinionStatus::Active)
    {
        if op.subject.trim().is_empty() {
            continue;
        }
        match newest_by_subject.get(&op.subject) {
            Some(_older) => {
                let older_id = newest_by_subject[&op.subject].clone();
                out.push(Drift {
                    older_id,
                    by_id: op.id.clone(),
                    subject: op.subject.clone(),
                });
                newest_by_subject.insert(op.subject.clone(), op.id.clone());
            }
            None => {
                newest_by_subject.insert(op.subject.clone(), op.id.clone());
            }
        }
    }
    out
}

/// Run the opinion-drift pass: detect same-subject contradictions and emit
/// `SupersedeOpinion` actions through the gate for each. Returns how many events
/// it appended.
fn opinion_drift(state: &AppState) -> Result<u32> {
    let projection = state.projection()?;
    let mut authored = 0u32;

    // A single cause drives correlation; like the PM loop.
    let latest = state.store.latest_sequence(&state.project)?;
    let cause = state
        .store
        .read_since(&state.project, latest.saturating_sub(1))?
        .last()
        .cloned();
    let correlation = format!("reconcile-{}", latest);

    for d in drift(&projection) {
        let action = PmAction::SupersedeOpinion {
            opinion_id: d.older_id,
            by_opinion_id: d.by_id,
        };
        // Validate against a fresh projection (so an earlier supersede is
        // visible); skip if the gate rejects.
        let projection = state.projection()?;
        let who = "system"; // reconciler acts as the system, not a human/agent
        if let Err(e) = crate::actions::validate(&action, who, &projection) {
            eprintln!("[reconciler] gate rejected {action:?}: {e}");
            continue;
        }
        let events = match &cause {
            Some(c) => action.to_events(&state.project, who, c, &correlation),
            None => Vec::new(),
        };
        for event in events {
            state.append(event.clone())?;
            authored += 1;
        }
    }
    Ok(authored)
}

/// Archive terminal entities: done tasks, superseded decisions/opinions,
/// resolved risks, and inactive observations. Fires an `EntityArchived` event
/// for each so the projection folds them out of the active lists and into the
/// compact history. Reduces agent context bloat. The event log retains the
/// full history for provenance. Returns how many were archived.
pub fn archive_terminals(state: &AppState) -> Result<u32> {
    use crate::event::{Actor, Aggregate, Event, EventType};
    use crate::projection::{DecisionStatus, OpinionStatus, RiskStatus, TaskStatus};

    let projection = state.projection()?;
    let mut archived_count = 0u32;

    // Done tasks.
    for task in projection.tasks.iter().filter(|t| t.status == TaskStatus::Done) {
        let reviewed = task
            .review
            .as_ref()
            .map(|r| {
                if r.approved {
                    format!(" (approved by {})", r.reviewer)
                } else {
                    String::new()
                }
            })
            .unwrap_or_default();
        let summary = format!("task \"{}\" ({}) — done{reviewed}", task.title, task.kind);
        archived_count += emit_archive(state, "task", &task.id, &summary, "done")?;
    }

    // Superseded decisions.
    for d in projection
        .decisions
        .iter()
        .filter(|d| d.status == DecisionStatus::Superseded)
    {
        let summary = format!("decision \"{}\" — superseded", d.subject);
        archived_count += emit_archive(state, "decision", &d.id, &summary, "superseded")?;
    }

    // Superseded opinions.
    for o in projection
        .opinions
        .iter()
        .filter(|o| o.status == OpinionStatus::Superseded)
    {
        let summary = format!("opinion \"{}\" — superseded", o.subject);
        archived_count += emit_archive(state, "opinion", &o.id, &summary, "superseded")?;
    }

    // Resolved risks.
    for r in projection.risks.iter().filter(|r| r.status == RiskStatus::Resolved) {
        let summary = format!("risk \"{}\" — resolved", r.subject);
        archived_count += emit_archive(state, "risk", &r.id, &summary, "resolved")?;
    }

    Ok(archived_count)
}

/// Append a single `EntityArchived` event for one terminal entity.
fn emit_archive(
    state: &AppState,
    kind: &str,
    id: &str,
    summary: &str,
    result: &str,
) -> Result<u32> {
    use crate::event::{Actor, Aggregate, Event, EventType};
    // The aggregate id is the archived "record" (distinct from the entity itself).
    let ev = Event::new(
        &state.project,
        Actor::System,
        EventType::EntityArchived,
        Aggregate {
            kind: "archive".into(),
            id: format!("arch-{kind}-{id}"),
        },
        serde_json::json!({
            "entity_kind": kind,
            "entity_id": id,
            "summary": summary,
            "result": result,
            "archived_by": "reconciler",
        }),
    );
    state.append(ev)?;
    Ok(1)
}

/// Prune isolated worktrees whose task is no longer active (Done, or whose
/// ChangeSet is Merged). Structural-isolation lifecycle close (2026-08-12):
/// once a consultant's work is complete/merged, their desk is torn down —
/// physically (`Workspace.remove_worktree`) and in the projection (a
/// `WorktreeRemoved` event, which frees the worktree's port for reuse). The
/// `WorktreeProvisioned` event remains as history. Returns how many were pruned.
pub fn prune_worktrees(state: &AppState) -> Result<u32> {
    use crate::event::{Actor, Aggregate, Event, EventType};
    use crate::projection::ChangeSetStatus;
    use crate::projection::TaskStatus;

    let projection = state.projection()?;
    let mut pruned = 0u32;
    let ws = match &state.workspace {
        Some(ws) => ws.clone(),
        // No workspace attached → nothing physical to prune (tests without a repo).
        None => return Ok(0),
    };

    let done_tasks: std::collections::HashSet<String> = projection
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Done)
        .map(|t| t.id.clone())
        .collect();
    let merged_changesets: std::collections::HashSet<String> = projection
        .changesets
        .iter()
        .filter(|c| c.status == ChangeSetStatus::Merged)
        .map(|c| c.task_id.clone())
        .collect();

    // A worktree is prunable if it has a bound task that is Done OR its ChangeSet is Merged.
    let prunable: Vec<String> = projection
        .worktrees
        .iter()
        .filter(|w| {
            w.task_id
                .as_ref()
                .is_some_and(|tid| done_tasks.contains(tid) || merged_changesets.contains(tid))
        })
        .map(|w| w.task_id.clone().unwrap_or_default())
        .collect();

    for task_id in prunable {
        // Physical teardown first (idempotent; missing tree is fine).
        let _ = ws.remove_worktree(&task_id);
        // Record the lifecycle close in the event log so the projection drops
        // the Worktree (freeing its port). Advisory-at-write: no precondition.
        let latest = state.store.latest_sequence(&state.project)?;
        let cause = state
            .store
            .read_since(&state.project, latest.saturating_sub(1))?
            .last()
            .cloned();
        let base_cause = Event::new(
            &state.project,
            Actor::System,
            EventType::WorktreeRemoved,
            Aggregate {
                kind: "worktree".into(),
                id: format!("wt-{task_id}"),
            },
            serde_json::json!({ "task_id": task_id }),
        );
        state.append(match cause {
            Some(c) => {
                let mut ev = base_cause;
                ev.metadata = crate::event::Metadata {
                    causation_id: Some(c.event_id),
                    ..Default::default()
                };
                ev
            }
            None => base_cause,
        })?;
        pruned += 1;
    }
    Ok(pruned)
}

/// Run every registered reconciliation pass now, then advance the cursor to the
/// (possibly grown) head so "every N events" stays fresh. Returns total events
/// appended across all passes.
pub fn run_passes(state: &AppState) -> Result<u32> {
    let mut total = 0u32;
    for pass in &state.reconcile_passes {
        match pass.run(state) {
            Ok(n) => {
                if n > 0 {
                    eprintln!("[reconciler] pass {} authored {n} event(s)", pass.name());
                }
                total += n;
            }
            Err(e) => eprintln!("[reconciler] pass {} error: {e:#}", pass.name()),
        }
    }
    let head = state.store.latest_sequence(&state.project)?;
    state
        .cursors
        .advance(&state.project, RECONCILER_CONSUMER, head)?;
    Ok(total)
}

/// Run the reconciler if due; else no-op. Convenience wrapper for the loop.
pub fn run_if_due(state: &AppState) -> Result<u32> {
    if should_run(state)? {
        run_passes(state)
    } else {
        Ok(0)
    }
}
