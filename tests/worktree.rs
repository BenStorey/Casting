//! Tests for worktree provisioning — each summoned consultant's isolated
//! workspace: a git worktree on its own branch, a private Rust build target,
//! and a distinct API port so concurrent consultants can't collide (owner
//! requirements 2026-08-12).
//!
//! All use throwaway repos under tempdir; none touch the real artifact repo.

use casting::workspace::{ProvisionedWorktree, Selfhost, Workspace};
use std::path::Path;

/// A fresh, existing `repo` dir inside a tempdir (state collocated in
/// `<repo>/.casting/`), with a real git repo and one commit so worktrees can
/// branch off HEAD.
fn repo_dir() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let ws = Workspace::open(&repo, Selfhost::Disabled).unwrap();
    ws.ensure_repo().unwrap();
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    ws.git_command().arg("add").arg(".").output().unwrap();
    ws.git_command()
        .arg("commit")
        .arg("-m")
        .arg("initial")
        .output()
        .unwrap();
    (tmp, repo)
}

fn ws(repo: &Path) -> Workspace {
    Workspace::open(repo, Selfhost::Disabled).unwrap()
}

#[test]
fn provision_creates_an_isolated_worktree_on_its_own_branch() {
    let (tmp, repo) = repo_dir();
    let _tmp = tmp;
    let ws = ws(&repo);

    let wt = ws
        .provision_worktree("task-381", "authentication", 8090)
        .unwrap();

    // Branch follows the casting/task-<id>-<slug> convention.
    assert_eq!(wt.branch, "casting/task-381-authentication");
    // Worktree lives under <repo>/.casting/worktrees/<task_id> (self-ignored).
    assert_eq!(wt.path, repo.join(".casting/worktrees/task-381"));
    assert!(wt.path.exists(), "worktree dir must exist");

    // The worktree's checked-out branch is its own (not main).
    let branch = ws
        .git_command_for(&wt.path)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()
        .unwrap();
    let branch = String::from_utf8_lossy(&branch.stdout).trim().to_string();
    assert_eq!(branch, "casting/task-381-authentication");
}

#[test]
fn each_worktree_gets_a_private_build_target() {
    let (tmp, repo) = repo_dir();
    let _tmp = tmp;
    let ws = ws(&repo);

    let a = ws
        .provision_worktree("task-381", "authentication", 8090)
        .unwrap();
    let b = ws.provision_worktree("task-382", "billing", 8091).unwrap();

    // Distinct CARGO_TARGET_DIRs, each inside its own worktree.
    assert_ne!(a.cargo_target_dir, b.cargo_target_dir);
    assert_eq!(a.cargo_target_dir, a.path.join("target"));
    assert_eq!(b.cargo_target_dir, b.path.join("target"));
    assert!(a.cargo_target_dir.starts_with(&a.path));
    assert!(b.cargo_target_dir.starts_with(&b.path));
}

#[test]
fn each_worktree_gets_a_distinct_port() {
    let (tmp, repo) = repo_dir();
    let _tmp = tmp;
    let ws = ws(&repo);

    let a = ws
        .provision_worktree("task-381", "authentication", 8090)
        .unwrap();
    let b = ws.provision_worktree("task-382", "billing", 8091).unwrap();

    assert_ne!(
        a.port, b.port,
        "concurrent consultants must not share a port"
    );
    assert_eq!(a.port, 8090);
    assert_eq!(b.port, 8091);
}

#[test]
fn worktrees_do_not_touch_main_or_the_shared_checkout() {
    let (tmp, repo) = repo_dir();
    let _tmp = tmp;
    let ws = ws(&repo);
    let main_head = ws.head().unwrap();

    ws.provision_worktree("task-381", "authentication", 8090)
        .unwrap();

    // Protected branch (main) is untouched.
    assert_eq!(ws.head().unwrap(), main_head, "HEAD must not move");

    // A change made inside the worktree is isolated: it does not appear in the
    // shared checkout's working tree.
    let wt = ws.worktree_path("task-381");
    std::fs::write(wt.join("change.txt"), "in worktree\n").unwrap();
    let shared = ws
        .git_command()
        .arg("status")
        .arg("--porcelain")
        .output()
        .unwrap();
    let shared_out = String::from_utf8_lossy(&shared.stdout);
    assert!(
        !shared_out.contains("change.txt"),
        "worktree change must not leak into the shared checkout: {shared_out}"
    );
}

#[test]
fn provision_is_idempotent_per_task() {
    let (tmp, repo) = repo_dir();
    let _tmp = tmp;
    let ws = ws(&repo);

    let a = ws
        .provision_worktree("task-381", "authentication", 8090)
        .unwrap();
    let b = ws
        .provision_worktree("task-381", "authentication", 8090)
        .unwrap();

    assert_eq!(a.path, b.path, "same task reuses the same worktree path");
    assert_eq!(a.branch, b.branch);
}

#[test]
fn remove_worktree_deletes_the_worktree() {
    let (tmp, repo) = repo_dir();
    let _tmp = tmp;
    let ws = ws(&repo);

    let wt = ws
        .provision_worktree("task-381", "authentication", 8090)
        .unwrap();
    assert!(wt.path.exists());

    ws.remove_worktree("task-381").unwrap();
    assert!(
        !wt.path.exists(),
        "worktree should be removed after cleanup"
    );
    // Idempotent: removing again is fine.
    ws.remove_worktree("task-381").unwrap();
}

#[test]
fn provisioned_worktree_is_structurally_correct() {
    let wt = ProvisionedWorktree {
        task_id: "task-9".into(),
        branch: "casting/task-9-auth".into(),
        path: Path::new("/x/wt").to_path_buf(),
        cargo_target_dir: Path::new("/x/wt/target").to_path_buf(),
        port: 9000,
    };
    assert_eq!(wt.task_id, "task-9");
    assert!(wt.cargo_target_dir.starts_with(&wt.path));
    assert_eq!(wt.port, 9000);
}

/// A WorktreeProvisioned event projects into `Projection.worktrees` and
/// auto-creates the Open ChangeSet with the exact branch mapping (no
/// branch-name guessing).
#[test]
fn worktree_provisioned_event_projects_worktree_and_change_set() {
    use casting::event::{Actor, Aggregate, Event, EventType};
    use casting::projection::{ChangeSetStatus, Projection};
    use casting::sqlite_store::SqliteEventStore;
    use casting::store::EventStore;

    let dir = tempfile::tempdir().unwrap();
    let store = SqliteEventStore::open(dir.path().join("events.db")).unwrap();
    store
        .append(Event::new(
            "proj",
            Actor::System,
            EventType::WorktreeProvisioned,
            Aggregate {
                kind: "worktree".into(),
                id: "wt-task-381".into(),
            },
            serde_json::json!({
                "task_id": "task-381",
                "branch": "casting/task-381-authentication",
                "path": "/repo/.casting/worktrees/task-381",
                "cargo_target_dir": "/repo/.casting/worktrees/task-381/target",
                "port": 8090,
            }),
        ))
        .unwrap();

    let proj = Projection::build(&store, "proj").unwrap();
    assert_eq!(proj.worktrees.len(), 1);
    let wt = proj.worktrees.first().unwrap();
    assert_eq!(wt.task_id, "task-381");
    assert_eq!(wt.branch, "casting/task-381-authentication");
    assert_eq!(wt.port, 8090);
    assert!(wt.cargo_target_dir.ends_with("target"));

    // Auto-created Open ChangeSet with the exact branch + task mapping.
    let cs = proj
        .changesets
        .iter()
        .find(|c| c.task_id == "task-381")
        .unwrap();
    assert_eq!(cs.id, "changeset-task-381");
    assert_eq!(cs.branch, "casting/task-381-authentication");
    assert_eq!(cs.status, ChangeSetStatus::Open);
    assert!(cs.commits.is_empty());
}

/// PM onboarding plans a ProvisionWorktree for each consultant-assigned task,
/// allocating distinct ports, so StartTask can pass the fail-closed gate.
#[tokio::test]
async fn pm_onboarding_provisions_distinct_worktrees() {
    use casting::cursor::CursorStore as _;
    use casting::pm::AppState;
    use casting::store::EventStore as _;
    use std::time::Duration;

    let store = casting::sqlite_store::SqliteEventStore::in_memory().unwrap();
    let cursors = casting::cursor::SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj").with_step_delay(Duration::ZERO);

    // Hire the default cast + seed project so plan_onboard sees requirements
    // empty and kicks off (mirrors the vertical-slice happy path but checks
    // worktrees). Drive one owner message.
    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::System,
            casting::event::EventType::ProjectCreated,
            casting::event::Aggregate {
                kind: "project".into(),
                id: "proj".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    for (id, role) in [
        ("pm", "Project Manager"),
        ("marcus-reed", "Engineer"),
        ("maya-patel", "QA"),
    ] {
        state
            .append(casting::event::Event::new(
                "proj",
                casting::event::Actor::System,
                casting::event::EventType::AgentHired,
                casting::event::Aggregate {
                    kind: "agent".into(),
                    id: id.into(),
                },
                serde_json::json!({ "role": role }),
            ))
            .unwrap();
    }
    // The PM's cursor must start before the seed events so it reacts to the
    // owner message (not just seeds). Advance it past what we appended.
    state
        .cursors
        .advance("proj", "pm", state.store.latest_sequence("proj").unwrap())
        .unwrap();

    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::Owner,
            casting::event::EventType::MessageSent,
            casting::event::Aggregate {
                kind: "message".into(),
                id: "m1".into(),
            },
            serde_json::json!({ "body": "Build me an app" }),
        ))
        .unwrap();

    let authored = casting::pm::drive_pm(&state).await.unwrap();
    assert!(authored > 0, "PM should author onboarding work");

    let proj = state.projection().unwrap();
    // Consultant-assigned tasks got worktrees with DISTINCT ports.
    assert!(
        !proj.worktrees.is_empty(),
        "onboarding should provision worktrees"
    );
    let ports: std::collections::HashSet<u16> = proj.worktrees.iter().map(|w| w.port).collect();
    assert_eq!(
        ports.len(),
        proj.worktrees.len(),
        "worktrees must have distinct ports"
    );
    // Each consultant task has its own branch in the casting/task-* convention.
    for wt in &proj.worktrees {
        assert!(wt.branch.starts_with("casting/task-"), "branch {wt:?}");
    }
    // Fail-closed StartTask would have been rejected if a worktree were missing,
    // so the fact onboarding completed proves every StartTask had a worktree.
}

/// Full path with a REAL workspace: drive the PM against a real git repo and
/// confirm consultant worktrees are physically created (own dir + branch), with
/// distinct ports.
#[tokio::test]
async fn pm_physically_provisions_worktrees_with_workspace() {
    use casting::cursor::CursorStore as _;
    use casting::pm::AppState;
    use casting::store::EventStore as _;
    use std::sync::Arc;
    use std::time::Duration;

    let (_tmp, repo) = repo_dir(); // real git repo with an initial commit
    let ws = ws(&repo);
    let state = {
        let store = casting::sqlite_store::SqliteEventStore::in_memory().unwrap();
        let cursors = casting::cursor::SqliteCursorStore::in_memory().unwrap();
        AppState::new(store, cursors, "proj")
            .with_step_delay(Duration::ZERO)
            .with_workspace(Arc::new(ws.clone()))
    };

    // Seed + owner message (same as above).
    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::System,
            casting::event::EventType::ProjectCreated,
            casting::event::Aggregate {
                kind: "project".into(),
                id: "proj".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    for (id, role) in [
        ("pm", "Project Manager"),
        ("marcus-reed", "Engineer"),
        ("maya-patel", "QA"),
    ] {
        state
            .append(casting::event::Event::new(
                "proj",
                casting::event::Actor::System,
                casting::event::EventType::AgentHired,
                casting::event::Aggregate {
                    kind: "agent".into(),
                    id: id.into(),
                },
                serde_json::json!({ "role": role }),
            ))
            .unwrap();
    }
    state
        .cursors
        .advance("proj", "pm", state.store.latest_sequence("proj").unwrap())
        .unwrap();
    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::Owner,
            casting::event::EventType::MessageSent,
            casting::event::Aggregate {
                kind: "message".into(),
                id: "m1".into(),
            },
            serde_json::json!({ "body": "Build me an app" }),
        ))
        .unwrap();

    casting::pm::drive_pm(&state).await.unwrap();

    let proj = state.projection().unwrap();
    assert!(!proj.worktrees.is_empty());
    for wt in &proj.worktrees {
        // The worktree dir physically exists on its own branch.
        let path = std::path::Path::new(&wt.path);
        assert!(path.exists(), "worktree dir {} should exist", wt.path);
        let branch = ws
            .git_command_for(path)
            .arg("rev-parse")
            .arg("--abbrev-ref")
            .arg("HEAD")
            .output()
            .unwrap();
        let branch = String::from_utf8_lossy(&branch.stdout).trim().to_string();
        assert_eq!(branch, wt.branch, "worktree should be on its own branch");
        // Private build target inside the worktree.
        assert!(Path::new(&wt.cargo_target_dir).starts_with(path));
    }
}

/// The agent git surface end-to-end: provision a worktree, write a file in it,
/// commit via the workspace, and confirm the commit landed ON the worktree's
/// own branch (never the shared checkout).
#[test]
fn commit_in_worktree_lands_on_the_isolated_branch() {
    use casting::workspace::Selfhost;
    use std::path::Path;

    let (tmp, repo) = repo_dir();
    let _tmp = tmp;
    let ws = Workspace::open(&repo, Selfhost::Disabled).unwrap();
    let wt = ws
        .provision_worktree("task-381", "authentication", 8090)
        .unwrap();
    let main_head = ws.head().unwrap();

    // The consultant writes a file inside their worktree, then commits.
    std::fs::write(wt.path.join("auth.rs"), "fn auth() {}\n").unwrap();
    ws.commit_in_worktree("task-381", "add auth module")
        .unwrap();

    // The commit landed on the worktree's branch (not main).
    let branch_log = ws
        .git_command_for(&wt.path)
        .arg("log")
        .arg("--oneline")
        .arg("-1")
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&branch_log.stdout).to_string();
    assert!(
        log.contains("add auth module"),
        "commit should exist on worktree branch, got: {log}"
    );
    // main is untouched — the work belongs only to this consultant's branch.
    assert_eq!(
        ws.head().unwrap(),
        main_head,
        "main must not move when the worktree commits"
    );
    assert!(Path::new(&wt.cargo_target_dir).starts_with(&wt.path));
}
