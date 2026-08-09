//! Integration tests for the git observer (Git slice increment 2).
//!
//! The observer turns raw repo state (branches, commits, merges) into semantic
//! domain events (`BranchCreated`, `CommitObserved`, `MergeCompleted`) via the
//! event store, using a durable cursor (same shape as the PM loop). These tests
//! prove the full pipeline: git operations -> observe() -> events -> projection.
//!
//! All repos are throwaway tempdirs; none ever touch the product repo.

use casting::cursor::CursorStore;
use casting::event::EventType;
use casting::git_observer;
use casting::projection::Projection;
use casting::sqlite_store::SqliteEventStore;
use casting::store::EventStore;
use casting::workspace::{Selfhost, Workspace};

/// A fresh workspace with a real git repo, ready for observer tests.
fn ws_with_repo() -> (tempfile::TempDir, Workspace, SqliteEventStore, CursorStore) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let state = tmp.path().join("state");

    let ws = Workspace::open(&repo, &state, Selfhost::Disabled).unwrap();
    ws.ensure_repo().unwrap();

    let store = SqliteEventStore::open(ws.state_dir.join("events.db")).unwrap();
    let cursors = CursorStore::open(ws.state_dir.join("cursors.db")).unwrap();

    (tmp, ws, store, cursors)
}

/// Helper: make a commit on the current branch.
fn commit(ws: &Workspace, msg: &str) {
    // Use a unique filename so each commit changes something.
    let name = format!("file-{}.txt", uuid::Uuid::new_v4());
    std::fs::write(ws.repo.join(&name), "content\n").unwrap();
    ws.git_command().arg("add").arg(&name).output().unwrap();
    ws.git_command()
        .arg("commit")
        .arg("-m")
        .arg(msg)
        .output()
        .unwrap();
}

/// Helper: create a branch (optionally with a commit).
fn branch(ws: &Workspace, name: &str, with_commit: bool) {
    ws.git_command()
        .arg("checkout")
        .arg("-b")
        .arg(name)
        .output()
        .unwrap();
    if with_commit {
        commit(ws, &format!("work on {name}"));
    }
}

#[test]
fn observer_emits_branch_created_for_new_branches() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    // Make an initial commit on the default branch, then create a feature branch.
    commit(&ws, "initial");
    branch(&ws, "casting/task-381-authentication", false);

    let emitted = git_observer::observe(&ws, &store, &cursors, "proj").unwrap();
    assert!(emitted >= 2, "should emit for main + feature branch, got {emitted}");

    let proj = Projection::build(&store, "proj").unwrap();
    assert!(
        proj.branches.iter().any(|b| b.name == "casting/task-381-authentication"),
        "feature branch should be in projection"
    );
    assert!(
        proj.branches.iter().any(|b| b.name == "main" || b.name == "master"),
        "default branch should be in projection"
    );
    // The task_id should be derived from the branch name.
    let feature = proj
        .branches
        .iter()
        .find(|b| b.name == "casting/task-381-authentication")
        .unwrap();
    assert_eq!(feature.task_id.as_deref(), Some("task-381"));
}

#[test]
fn observer_emits_commit_observed_for_new_commits() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    commit(&ws, "first commit");
    commit(&ws, "second commit");

    let emitted = git_observer::observe(&ws, &store, &cursors, "proj").unwrap();
    assert!(emitted >= 3, "should emit 1 branch + 2 commits, got {emitted}");

    let proj = Projection::build(&store, "proj").unwrap();
    assert_eq!(
        proj.commits.len(),
        2,
        "two commits should be in projection"
    );
    let first = proj
        .commits
        .iter()
        .find(|c| c.message == "first commit")
        .unwrap();
    assert!(!first.sha.is_empty());
    assert!(!first.author.is_empty());
}

#[test]
fn observer_is_idempotent_on_replay() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    commit(&ws, "first");
    let first = git_observer::observe(&ws, &store, &cursors, "proj").unwrap();
    assert!(first > 0, "first pass should emit events");

    // Second pass with no new commits should emit nothing.
    let second = git_observer::observe(&ws, &store, &cursors, "proj").unwrap();
    assert_eq!(second, 0, "no new events on replay");

    let proj = Projection::build(&store, "proj").unwrap();
    assert_eq!(proj.commits.len(), 1, "still one commit");
    assert_eq!(proj.branches.len(), 1, "still one branch");
}

#[test]
fn observer_picks_up_new_commits_on_second_pass() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    commit(&ws, "first");
    git_observer::observe(&ws, &store, &cursors, "proj").unwrap();

    // Add a new commit.
    commit(&ws, "second");
    let emitted = git_observer::observe(&ws, &store, &cursors, "proj").unwrap();
    assert_eq!(emitted, 1, "should emit one new CommitObserved");

    let proj = Projection::build(&store, "proj").unwrap();
    assert_eq!(proj.commits.len(), 2, "two commits now");
}

#[test]
fn observer_emits_merge_completed_for_merge_commits() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    // Create main with a commit, then a feature branch with a commit.
    commit(&ws, "on main");
    // Detect the default branch name (could be main or master).
    let default_branch = ws.current_branch().unwrap();
    branch(&ws, "casting/task-50-feature", true);

    // Merge the feature branch back into the default branch.
    ws.git_command()
        .arg("checkout")
        .arg(&default_branch)
        .output()
        .unwrap();
    ws.git_command()
        .arg("merge")
        .arg("--no-ff")
        .arg("casting/task-50-feature")
        .arg("-m")
        .arg("merge feature")
        .output()
        .unwrap();

    let emitted = git_observer::observe(&ws, &store, &cursors, "proj").unwrap();
    assert!(
        emitted >= 5,
        "should emit branches + commits + merge, got {emitted}"
    );

    let proj = Projection::build(&store, "proj").unwrap();
    assert!(
        !proj.merges.is_empty(),
        "at least one merge should be in projection"
    );
    let merge = &proj.merges[0];
    assert_eq!(merge.to_branch, default_branch);
    assert!(!merge.sha.is_empty());
}

#[test]
fn observer_handles_empty_repo() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;
    let _ = ws;

    // No commits, no branches — observe should succeed and emit nothing.
    let emitted = git_observer::observe(&ws, &store, &cursors, "proj").unwrap();
    assert_eq!(emitted, 0, "empty repo should emit no events");
}

#[test]
fn observer_cursor_advances_durably() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    commit(&ws, "first");
    git_observer::observe(&ws, &store, &cursors, "proj").unwrap();

    // The cursor should have advanced.
    let cursor = cursors.get("proj", git_observer::GIT_OBSERVER_CONSUMER).unwrap();
    assert!(cursor.last_seen > 0, "cursor should have advanced");
    assert_eq!(
        cursor.last_seen,
        store.latest_sequence("proj").unwrap(),
        "cursor should match latest sequence"
    );
}

#[test]
fn derive_task_id_from_branch_name() {
    // The derive_task_id function is private, but we test it indirectly through
    // the BranchCreated event's task_id field (tested above). Here we just
    // confirm the convention works end-to-end with a multi-segment branch name.
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    commit(&ws, "initial");
    branch(&ws, "casting/task-999-some-long-feature-name", false);

    git_observer::observe(&ws, &store, &cursors, "proj").unwrap();
    let proj = Projection::build(&store, "proj").unwrap();
    let branch = proj
        .branches
        .iter()
        .find(|b| b.name == "casting/task-999-some-long-feature-name")
        .unwrap();
    assert_eq!(branch.task_id.as_deref(), Some("task-999"));
}

#[test]
fn changeset_auto_derived_from_task_branch() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    commit(&ws, "initial");
    branch(&ws, "casting/task-100-feature", true);

    git_observer::observe(&ws, &store, &cursors, "proj").unwrap();
    let proj = Projection::build(&store, "proj").unwrap();

    // An Open ChangeSet should be auto-derived from the task branch.
    let cs = proj
        .changesets
        .iter()
        .find(|c| c.task_id == "task-100")
        .expect("ChangeSet should be auto-derived for task-100");
    assert_eq!(cs.branch, "casting/task-100-feature");
    assert_eq!(cs.status, casting::projection::ChangeSetStatus::Open);
    // All commits reachable from the branch are linked to the ChangeSet
    // (including those inherited from the parent branch — git log shows
    // the full reachable history).
    assert!(!cs.commits.is_empty(), "at least one commit should be linked");
}

#[test]
fn changeset_ready_event_updates_status() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    // Set up a branch with a commit -> observe -> auto-derive ChangeSet.
    commit(&ws, "initial");
    branch(&ws, "casting/task-200-work", true);
    git_observer::observe(&ws, &store, &cursors, "proj").unwrap();

    // Emit a ChangeSetReady event (as the PM/agent would).
    let ev = casting::event::Event::new(
        "proj",
        casting::event::Actor::System,
        EventType::ChangeSetReady,
        casting::event::Aggregate {
            kind: "changeset".into(),
            id: "changeset-task-200".into(),
        },
        serde_json::json!({
            "task_id": "task-200",
            "branch": "casting/task-200-work",
            "commits": [],
            "agent": "marcus-reed"
        }),
    );
    store.append(ev).unwrap();

    let proj = Projection::build(&store, "proj").unwrap();
    let cs = proj
        .changesets
        .iter()
        .find(|c| c.id == "changeset-task-200")
        .unwrap();
    assert_eq!(cs.status, casting::projection::ChangeSetStatus::Ready);
    assert_eq!(cs.agent.as_deref(), Some("marcus-reed"));
}

#[test]
fn merge_marks_changeset_as_merged() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    commit(&ws, "initial");
    let default_branch = ws.current_branch().unwrap();
    branch(&ws, "casting/task-300-feature", true);

    // Merge the feature branch back.
    ws.git_command()
        .arg("checkout")
        .arg(&default_branch)
        .output()
        .unwrap();
    ws.git_command()
        .arg("merge")
        .arg("--no-ff")
        .arg("casting/task-300-feature")
        .arg("-m")
        .arg("merge feature")
        .output()
        .unwrap();

    git_observer::observe(&ws, &store, &cursors, "proj").unwrap();

    let proj = Projection::build(&store, "proj").unwrap();
    let cs = proj
        .changesets
        .iter()
        .find(|c| c.task_id == "task-300")
        .expect("ChangeSet should exist for task-300");
    assert_eq!(
        cs.status,
        casting::projection::ChangeSetStatus::Merged,
        "ChangeSet should be Merged after the merge completes"
    );
}