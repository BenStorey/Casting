//! The drift reconciler — a cursor-gated "every N events" cleanup pass.
//!
//! Knowledge accumulates and *drifts*: opinions don't go stale in a burst, they
//! slowly become inconsistent. Write-time supersession is eager and brittle
//! (the writer must know every target). Instead (owner framing 2026-08-10):
//! **keep writes simple; reconcile periodically.** This is a reusable primitive
//! — the same cursor-gated trigger will later drive priority/plan re-ranking.
//!
//! It runs as its OWN consumer (mirrors the PM loop): a `RECONCILER_CONSUMER`
//! cursor + a threshold interval. When `latest - reconciler_cursor >= N`, it
//! wakes, detects drift mechanically from the projection, emits ordinary gate
//! actions, and advances its cursor. The "smart" judgment of *what* truly
//! conflicts stays a D2 seam (the LLM reviewer); this skeleton does the
//! mechanically-obvious cleanup deterministically.

use crate::actions::PmAction;
use crate::pm::AppState;
use crate::projection::{OpinionStatus, Projection};
use anyhow::Result;

/// The reconciler's durable position in the event stream.
pub const RECONCILER_CONSUMER: &str = "reconciler";

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

/// Run one reconciliation pass now (the "every N events" body). Detects drift,
/// emits `SupersedeOpinion` actions through the gate for each, and advances the
/// reconciler cursor to the latest sequence so the next pass is a fresh window.
/// Returns how many events it appended.
pub async fn reconcile(state: &AppState) -> Result<u32> {
    let projection = state.projection()?;
    let mut authored = 0u32;

    // A single cause drives correlation; like the PM loop.
    let latest = state.store.latest_sequence(&state.project)?;
    let cause = state
        .store
        .read_since(&state.project, latest.saturating_sub(1))?
        .last()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no events to reconcile"))?;
    let correlation = format!("reconcile-{}", latest);

    for d in drift(&projection) {
        let action = PmAction::SupersedeOpinion {
            opinion_id: d.older_id,
            by_opinion_id: d.by_id,
        };
        // Validate against a running projection (recompute each iteration so an
        // earlier supersede is visible; skip if the gate rejects — already done).
        let projection = state.projection()?;
        let who = "system"; // reconciler acts as the system, not a human/agent
        if let Err(e) = crate::actions::validate(&action, who, &projection) {
            eprintln!("[reconciler] gate rejected {action:?}: {e}");
            continue;
        }
        for event in action.to_events(&state.project, who, &cause, &correlation) {
            state.append(event.clone())?;
            authored += 1;
        }
    }

    // Cursor now at the (possibly grown) head, so "every N events" stays fresh.
    let head = state.store.latest_sequence(&state.project)?;
    state
        .cursors
        .advance(&state.project, RECONCILER_CONSUMER, head)?;
    Ok(authored)
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

    // A worktree is prunable if its task is Done OR its ChangeSet is Merged.
    let prunable: Vec<String> = projection
        .worktrees
        .iter()
        .filter(|w| done_tasks.contains(&w.task_id) || merged_changesets.contains(&w.task_id))
        .map(|w| w.task_id.clone())
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

/// Run the reconciler if due; else no-op. Convenience wrapper for the loop.
pub async fn run_if_due(state: &AppState) -> Result<u32> {
    if should_run(state)? {
        let drifted = reconcile(state).await?;
        // Structural-isolation lifecycle close: on the same pass, prune
        // worktrees whose task is done/merged (physical + event-based).
        let pruned = prune_worktrees(state)?;
        Ok(drifted + pruned)
    } else {
        Ok(0)
    }
}
