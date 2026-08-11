//! Git observer — turns raw repo state into semantic domain events.
//!
//! Per docs/ADDENDUM.md §23: "Semantic events, not plumbing events." The
//! observer watches the artifact repo through the pinned git runner
//! (`Workspace::git_command()`) and emits `BranchCreated`, `CommitObserved`,
//! and `MergeCompleted` events into the event store. It holds a durable
//! cursor (same shape as the PM loop) so it only processes what's new since
//! its last pass.
//!
//! The observer is deliberately a *polling* observer, not a filesystem watch
//! — it runs on the same wake/drain model as the PM (broadcast hint → drain
//! since cursor). This keeps it simple, deterministic, and testable. A real
//! `inotify`/`watchexec` hook can come later; the cursor model doesn't care.
//!
//! What the observer emits (ADDENDUM §23):
//!   - `BranchCreated` — a new branch appeared (name + optional task_id).
//!   - `CommitObserved` — a new commit on a known branch (sha + message + author).
//!   - `MergeCompleted` — a merge commit appeared on a protected branch.
//!
//! What it does NOT emit (ADDENDUM §23 — plumbing, not semantic):
//!   `git status`, `git checkout`, `git add`, `git fetch`, object creation.
//!
//! `MergeConflictDetected` is NOT emitted by the passive observer — it
//! requires *attempting* a merge (an active operation). The git runner emits
//! it when a merge command fails due to conflicts (increment 3+).

use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use crate::store::EventStore;
use crate::workspace::Workspace;
use anyhow::{Context, Result};
use serde_json::json;

/// The consumer id under which the git observer stores its cursor.
pub const GIT_OBSERVER_CONSUMER: &str = "git-observer";

/// A snapshot of one branch as seen by the observer. Used to diff against
/// the next poll.
#[derive(Debug, Clone)]
struct BranchTip {
    name: String,
}

/// Run one observation pass: discover branches and commits that are new since
/// the observer's cursor, emit semantic events, then advance the cursor.
/// Returns the number of events emitted.
///
/// The observer works by listing all branches and their tips, comparing them
/// to what it has already seen (based on the cursor position — the cursor
/// tracks the last event sequence the observer has processed, and the events
/// it emitted tell it which commits it has already recorded). On the first
/// pass, everything is new.
pub fn observe<S: EventStore>(
    ws: &Workspace,
    store: &S,
    cursors: &dyn crate::cursor::CursorStore,
    project: &str,
) -> Result<u32> {
    let cursor = cursors.get(project, GIT_OBSERVER_CONSUMER)?;
    // We don't use the cursor to skip git operations (git is fast for small
    // repos); we use it to avoid emitting *duplicate* events for commits we've
    // already recorded. The cursor tracks the last event sequence we authored.
    // We check the existing projection's commits to know what we've seen.
    let _ = cursor; // cursor is advanced at the end; the dedup is projection-based.

    let mut emitted = 0u32;

    // --- Discover branches ---
    let branches = list_branches(ws)?;
    for branch in &branches {
        // Emit BranchCreated for branches we haven't recorded yet.
        // We check by looking at whether a BranchCreated event exists for this
        // branch name. Since the projection is derived from the log, we can
        // check the projection. But we don't have the projection here (we'd
        // need to build it). Instead, we emit BranchCreated idempotently: the
        // aggregate id is the branch name, and the event store doesn't dedup
        // by aggregate id. So we need a lightweight check.
        //
        // For increment 2, we use a simpler approach: track seen branches in
        // the cursor's metadata. But the cursor only stores a sequence number.
        // So we emit BranchCreated only if the branch is new — we check by
        // reading back events we've emitted (since our cursor).
        if !branch_already_recorded(store, project, cursor.last_seen, &branch.name)? {
            let ev = Event::new(
                project,
                Actor::System,
                EventType::BranchCreated,
                Aggregate {
                    kind: "branch".into(),
                    id: branch.name.clone(),
                },
                json!({
                    "task_id": derive_task_id(&branch.name),
                }),
            );
            store.append(ev)?;
            emitted += 1;
        }
    }

    // --- Discover new commits on each branch ---
    for branch in &branches {
        let commits = list_new_commits(ws, store, project, &branch.name)?;
        for commit in commits {
            let ev = Event::new(
                project,
                Actor::System,
                EventType::CommitObserved,
                Aggregate {
                    kind: "commit".into(),
                    id: commit.sha.clone(),
                },
                json!({
                    "branch": branch.name,
                    "message": commit.message,
                    "author": commit.author,
                    "task_id": derive_task_id(&branch.name),
                }),
            );
            store.append(ev)?;
            emitted += 1;

            // Check if this is a merge commit — emit MergeCompleted if so.
            if commit.is_merge {
                let parents = commit.parents.unwrap_or_default();
                let from_branch = parents
                    .first()
                    .and_then(|sha| branch_name_for_commit(ws, sha))
                    .unwrap_or_else(|| "unknown".to_string());
                let ev = Event::new(
                    project,
                    Actor::System,
                    EventType::MergeCompleted,
                    Aggregate {
                        kind: "merge".into(),
                        id: commit.sha.clone(),
                    },
                    json!({
                        "from_branch": from_branch,
                        "to_branch": branch.name,
                    }),
                );
                store.append(ev)?;
                emitted += 1;
            }
        }
    }

    // Advance the cursor past everything we've emitted (and everything else).
    let latest = store.latest_sequence(project)?;
    cursors.advance(project, GIT_OBSERVER_CONSUMER, latest)?;

    Ok(emitted)
}

/// Check whether a BranchCreated event already exists for this branch name.
/// Reads the whole event log (since 0) — the cursor tracks what we've
/// processed, but dedup needs to see everything we've ever emitted.
fn branch_already_recorded<S: EventStore>(
    store: &S,
    project: &str,
    _after: i64,
    branch_name: &str,
) -> Result<bool> {
    let events = store.read_since(project, 0)?;
    Ok(events
        .iter()
        .any(|e| e.event_type == EventType::BranchCreated && e.aggregate.id == branch_name))
}

/// Derive a task id from a branch name following the `casting/task-N-*`
/// convention (ADDENDUM §20). Returns None if the branch doesn't follow it.
fn derive_task_id(branch_name: &str) -> Option<String> {
    // casting/task-381-authentication -> task-381
    let prefix = "casting/task-";
    if let Some(rest) = branch_name.strip_prefix(prefix) {
        if let Some(end) = rest.find('-') {
            return Some(format!("task-{}", &rest[..end]));
        }
        // No suffix: casting/task-381 -> task-381
        return Some(format!("task-{rest}"));
    }
    None
}

/// One commit's metadata, as extracted from `git log`.
struct CommitInfo {
    sha: String,
    message: String,
    author: String,
    is_merge: bool,
    /// Parent commit shas (for merge detection / from_branch).
    parents: Option<Vec<String>>,
}

/// List all branches in the repo.
fn list_branches(ws: &Workspace) -> Result<Vec<BranchTip>> {
    let out = ws
        .git_command()
        .arg("for-each-ref")
        .arg("--format=%(refname:short)")
        .arg("refs/heads/")
        .output()
        .context("git for-each-ref")?;
    if !out.status.success() {
        // No branches yet (fresh repo) — return empty.
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let branches = text
        .lines()
        .map(|name| BranchTip {
            name: name.trim().to_string(),
        })
        .filter(|b| !b.name.is_empty())
        .collect();
    Ok(branches)
}

/// List commits on `branch` that haven't been recorded as CommitObserved yet.
/// We compare against the projection's known commit shas.
fn list_new_commits<S: EventStore>(
    ws: &Workspace,
    store: &S,
    project: &str,
    branch: &str,
) -> Result<Vec<CommitInfo>> {
    // Get all commits on this branch (newest first).
    let out = ws
        .git_command()
        .arg("log")
        .arg("--format=%H|%s|%an|%P")
        .arg("--reverse") // oldest first — so we emit in chronological order
        .arg(branch)
        .output()
        .context("git log")?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&out.stdout);

    // Get the set of commit shas we've already recorded.
    let known_shas: std::collections::HashSet<String> = store
        .read_since(project, 0)?
        .iter()
        .filter(|e| e.event_type == EventType::CommitObserved)
        .map(|e| e.aggregate.id.clone())
        .collect();

    let commits = text
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 3 {
                return None;
            }
            let sha = parts[0].to_string();
            if known_shas.contains(&sha) {
                return None; // already recorded
            }
            let message = parts[1].to_string();
            let author = parts[2].to_string();
            let parents: Vec<String> = if parts.len() > 3 {
                parts[3].split_whitespace().map(String::from).collect()
            } else {
                Vec::new()
            };
            let is_merge = parents.len() > 1;
            Some(CommitInfo {
                sha,
                message,
                author,
                is_merge,
                parents: Some(parents),
            })
        })
        .collect();
    Ok(commits)
}

/// Resolve which branch name a commit sha belongs to (if any local branch
/// contains it as its tip or ancestor). Used to identify the `from_branch`
/// in a merge.
fn branch_name_for_commit(ws: &Workspace, sha: &str) -> Option<String> {
    let out = ws
        .git_command()
        .arg("branch")
        .arg("--contains")
        .arg(sha)
        .arg("--format=%(refname:short)")
        .output()
        .ok()?;
    if out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .map(|s| s.trim().to_string())
            .find(|s| !s.is_empty())
    } else {
        None
    }
}

/// Thin async wrapper around [`observe`] for use from the async runtime
/// (called at boot and on each PM drain). Reads/writes through `AppState`'s
/// store and cursors, broadcasts any new events so the UI/SSE updates live.
pub async fn observe_once(state: &AppState, ws: &Workspace) {
    let result = observe(ws, &state.store, &state.cursors, &state.project);
    match result {
        Ok(n) => {
            if n > 0 {
                // Broadcast the new events so SSE subscribers see them and the
                // PM is woken. Read back what we just appended (last n events).
                if let Ok(events) = state.store.read_since(&state.project, 0) {
                    for ev in events.iter().rev().take(n as usize).rev() {
                        state.notify(ev);
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("[git-observer] error: {e:#}");
        }
    }
}
