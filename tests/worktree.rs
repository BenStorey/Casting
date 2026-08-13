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

    // Verify the full worktree LIFECYCLE via the event log (the source of
    // truth): every task that was provisioned a worktree got one physically,
    // and every task that BECAME Done got it torn down immediately (write-time)
    // — both in the projection and on disk.
    let store: &dyn casting::store::EventStore = &state.store;
    let events = store.read_since("proj", 0).unwrap();
    let provisioned: std::collections::HashSet<String> = events
        .iter()
        .filter(|e| e.event_type == casting::event::EventType::WorktreeProvisioned)
        .filter_map(|e| {
            e.data
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();
    let removed: std::collections::HashSet<String> = events
        .iter()
        .filter(|e| e.event_type == casting::event::EventType::WorktreeRemoved)
        .filter_map(|e| {
            e.data
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect();

    assert!(
        !provisioned.is_empty(),
        "onboarding should have provisioned worktrees"
    );
    for tid in &provisioned {
        // If the task is Done, its worktree must be gone (write-time teardown).
        let task_done = state
            .projection()
            .unwrap()
            .tasks
            .iter()
            .any(|t| t.id == *tid && t.status == casting::projection::TaskStatus::Done);
        if task_done {
            assert!(removed.contains(tid), "done task {tid} should be torn down");
            assert!(
                !ws.worktree_path(tid).exists(),
                "done task {tid}'s worktree dir should be deleted"
            );
        }
    }

    // The in-progress (non-Done) worktrees that remain physically exist on their
    // own branch with a private build target.
    for wt in state.projection().unwrap().worktrees {
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

/// Reconciler lifecycle close: once a task is Done, its worktree is pruned
/// (physical dir removed + WorktreeRemoved event drops it and frees the port).
#[test]
fn reconciler_prunes_done_worktrees_and_frees_their_port() {
    use std::sync::Arc;

    let (_tmp, repo) = repo_dir();
    let ws = ws(&repo);
    // Provision two worktrees; one will be "done", one stays active.
    ws.provision_worktree("task-381", "authentication", 8090)
        .unwrap();
    ws.provision_worktree("task-382", "billing", 8091).unwrap();

    let store = casting::sqlite_store::SqliteEventStore::in_memory().unwrap();
    let cursors = casting::cursor::SqliteCursorStore::in_memory().unwrap();
    let state =
        casting::pm::AppState::new(store, cursors, "proj").with_workspace(Arc::new(ws.clone()));
    // Seed the event log with both WorktreeProvisioned so the projection has them.
    for (tid, port) in [("task-381", 8090), ("task-382", 8091)] {
        state
            .append(casting::event::Event::new(
                "proj",
                casting::event::Actor::System,
                casting::event::EventType::WorktreeProvisioned,
                casting::event::Aggregate {
                    kind: "worktree".into(),
                    id: format!("wt-{tid}"),
                },
                serde_json::json!({
                    "task_id": tid,
                    "branch": format!("casting/{tid}"),
                    "path": ws.worktree_path(tid).to_string_lossy().into_owned(),
                    "cargo_target_dir": ws.worktree_path(tid).join("target").to_string_lossy().into_owned(),
                    "port": port,
                }),
            ))
            .unwrap();
    }
    // Mark task-381 Done in the projection by seeding a Task + TaskCompleted.
    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::System,
            casting::event::EventType::TaskCreated,
            casting::event::Aggregate {
                kind: "task".into(),
                id: "task-381".into(),
            },
            serde_json::json!({ "title": "auth", "kind": "feature" }),
        ))
        .unwrap();
    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::System,
            casting::event::EventType::TaskCompleted,
            casting::event::Aggregate {
                kind: "task".into(),
                id: "task-381".into(),
            },
            serde_json::json!({ "result": "done" }),
        ))
        .unwrap();

    // Sanity: before pruning both worktrees exist in the projection and on disk.
    assert_eq!(state.projection().unwrap().worktrees.len(), 2);
    assert!(ws.worktree_path("task-381").exists());
    assert!(ws.worktree_path("task-382").exists());

    let pruned = casting::reconciler::prune_worktrees(&state).unwrap();
    assert_eq!(pruned, 1, "exactly the done task's worktree is pruned");

    let proj = state.projection().unwrap();
    // Only the active task's worktree remains; task-381's port is freed.
    assert_eq!(proj.worktrees.len(), 1);
    assert_eq!(proj.worktrees[0].task_id, "task-382");
    // The allocator returns the LOWEST free port in the pool — 8081 is lower
    // than 8090, but crucially 8090 is no longer taken (before pruning it was).
    let used: std::collections::HashSet<u16> = proj.worktrees.iter().map(|w| w.port).collect();
    assert!(
        !used.contains(&8090),
        "task-381's port 8090 must be freed for reuse"
    );
    // The remaining active worktree still holds its port (8091).
    assert!(used.contains(&8091));
    // The physical worktree dir is gone; the active one remains.
    assert!(!ws.worktree_path("task-381").exists());
    assert!(ws.worktree_path("task-382").exists());
}

/// The consultant's isolated workspace surfaces in the agent context AND the
/// operating picture (so D2/the owner can see the desk the agent works in).
#[test]
fn worktree_surfaces_in_context_and_operating_model() {
    use casting::context::WorktreeInfo;

    // Build a projection via the WorktreeProvisioned reducer.
    let (tmp, repo) = repo_dir();
    let _tmp = tmp;
    let ws = ws(&repo);
    // Build a projection directly (no store needed — pure derivation).
    let proj = casting::projection::Projection {
        project_id: "proj".into(),
        agents: vec![casting::projection::Agent {
            id: "marcus-reed".into(),
            role: "Engineer".into(),
        }],
        tasks: vec![casting::projection::Task {
            id: "task-381".into(),
            title: "auth".into(),
            kind: "feature".into(),
            status: casting::projection::TaskStatus::Backlog,
            assignee: Some("marcus-reed".into()),
            priority: casting::plan::Priority::default(),
            review: None,
        }],
        worktrees: vec![casting::projection::Worktree {
            task_id: "task-381".into(),
            branch: "casting/task-381".into(),
            path: ws.worktree_path("task-381").to_string_lossy().into_owned(),
            cargo_target_dir: ws
                .worktree_path("task-381")
                .join("target")
                .to_string_lossy()
                .into_owned(),
            port: 8090,
        }],
        ..Default::default()
    };

    // Agent context carries the desk.
    let ctx = proj.context_for("marcus-reed");
    assert!(ctx.my_tasks.contains(&"task-381".to_string()));
    let wt: WorktreeInfo = ctx.worktree.expect("consultant's worktree in context");
    assert_eq!(wt.task_id, "task-381");
    assert_eq!(wt.port, 8090);
    assert!(wt.cargo_target_dir.ends_with("/target"));

    // Operating picture includes the worktree view.
    let model = proj.operating_model();
    assert_eq!(model.worktrees.len(), 1);
    assert_eq!(model.worktrees[0].task_id, "task-381");
    assert_eq!(model.worktrees[0].port, 8090);
}
