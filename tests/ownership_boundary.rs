//! Tests for the ownership boundary (docs/OWNERSHIP_BOUNDARY.md, D5): the
//! self-identity guard, mandatory distinct state-dir, path sandboxing, and the
//! pinned git runner. These encode the safety invariant that Casting can never
//! conduct on the wrong repo — least of all the repo that built it.
//!
//! All git-like tests use throwaway repos under tempdir; none ever touch the
//! product repo at /home/ben/casting.

use casting::sqlite_store::SqliteEventStore;
use casting::store::EventStore;
use casting::workspace::{Selfhost, Workspace};
use std::path::Path;

/// Fresh, existing `repo` and `state` sibling dirs inside a tempdir.
fn sibling_dirs() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let state = tmp.path().join("state"); // created on demand by open()
    (tmp, repo, state)
}

#[test]
fn state_dir_must_be_distinct_and_non_nested_from_repo() {
    let (tmp, repo, _) = sibling_dirs();

    // Same path: rejected.
    assert!(Workspace::open(&repo, &repo, Selfhost::Disabled).is_err());

    // State inside the repo: rejected.
    let nested = repo.join(".casting");
    assert!(Workspace::open(&repo, &nested, Selfhost::Disabled).is_err());

    // Repo inside the state dir (state is the parent): rejected.
    assert!(Workspace::open(&repo, tmp.path(), Selfhost::Disabled).is_err());

    // Distinct siblings: accepted.
    let state = tmp.path().join("state");
    let ws = Workspace::open(&repo, &state, Selfhost::Disabled).unwrap();
    assert_ne!(ws.repo, ws.state_dir);
    assert!(!ws.state_dir.starts_with(&ws.repo));
    assert!(!ws.repo.starts_with(&ws.state_dir));
}

#[test]
fn refuses_the_repo_that_built_it() {
    let root = option_env!("CASTING_SOURCE_ROOT");
    let Some(root) = root else {
        eprintln!("CASTING_SOURCE_ROOT not set at build time; skipping");
        return;
    };
    let (retain, _, state) = sibling_dirs();
    let _ = retain;
    let err = Workspace::open(Path::new(root), &state, Selfhost::Disabled)
        .expect_err("should refuse the repo that built this binary");
    assert!(
        err.to_string().contains("Casting source"),
        "unexpected error: {err}"
    );
}

#[test]
fn refuses_a_repo_with_casting_identity() {
    let (retain, repo, state) = sibling_dirs();
    let _ = retain;
    // This repo is NOT the source root, but names the Casting crate.
    std::fs::write(repo.join("Cargo.toml"), "name = \"casting\"\n").unwrap();
    let err = Workspace::open(&repo, &state, Selfhost::Disabled)
        .expect_err("should refuse a repo naming the Casting crate");
    assert!(
        err.to_string().contains("Casting source"),
        "unexpected error: {err}"
    );
}

#[test]
fn selfhost_explicitly_enables_operating_on_a_casting_repo() {
    let (retain, repo, state) = sibling_dirs();
    let _ = retain;
    std::fs::write(repo.join("Cargo.toml"), "name = \"casting\"\n").unwrap();

    let ws = Workspace::open(&repo, &state, Selfhost::Enabled).unwrap();
    assert_eq!(ws.selfhost(), Selfhost::Enabled);
}

#[test]
fn resolve_under_resolves_inside_repo_and_rejects_escape() {
    let (retain, repo, state) = sibling_dirs();
    let _ = retain;
    let ws = Workspace::open(&repo, &state, Selfhost::Disabled).unwrap();

    // A normal relative path resolves under the repo.
    assert_eq!(
        ws.resolve_under(Path::new("src/main.rs")).unwrap(),
        repo.join("src/main.rs")
    );

    // `.` and nested `..` stay inside the repo.
    assert_eq!(
        ws.resolve_under(Path::new("./a/../a/b")).unwrap(),
        repo.join("a/b")
    );

    // Escapists and absolute inputs are refused.
    assert!(ws.resolve_under(Path::new("..")).is_err());
    assert!(ws.resolve_under(Path::new("../outside")).is_err());
    assert!(ws.resolve_under(Path::new("/etc/passwd")).is_err());
    assert!(ws.resolve_under(Path::new("/")).is_err());
}

#[test]
fn git_command_pins_the_repo_and_a_worktree_boundary() {
    let (retain, repo, state) = sibling_dirs();
    let _ = retain;
    let ws = Workspace::open(&repo, &state, Selfhost::Disabled).unwrap();

    let cmd = ws.git_command();
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(
        args.windows(2)
            .any(|w| w[0] == "-C" && Path::new(&w[1]) == ws.repo.as_path()),
        "git must run with -C <repo>, got args {args:?}"
    );

    let envs: Vec<_> = cmd
        .get_envs()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.map(|o| o.to_string_lossy().into_owned()),
            )
        })
        .collect();
    assert!(
        envs.iter()
            .any(|(k, v)| k == "GIT_WORK_TREE" && v.as_deref() == Some(repo.to_str().unwrap())),
        "GIT_WORK_TREE must pin the repo, got {envs:?}"
    );
    let git_dir = repo.join(".git");
    assert!(
        envs.iter()
            .any(|(k, v)| k == "GIT_DIR" && v.as_deref() == Some(git_dir.to_str().unwrap())),
        "GIT_DIR must pin <repo>/.git, got {envs:?}"
    );
}

#[test]
fn event_store_lives_in_state_dir_not_the_repo() {
    let (retain, repo, state) = sibling_dirs();
    let _ = retain;
    let ws = Workspace::open(&repo, &state, Selfhost::Disabled).unwrap();

    // Open the store via the workspace's state dir and append a real event.
    let store = SqliteEventStore::open(ws.state_dir.join("events.db")).unwrap();
    let ev = casting::event::Event::new(
        "proj",
        casting::event::Actor::System,
        casting::event::EventType::ProjectCreated,
        casting::event::Aggregate {
            kind: "project".into(),
            id: "proj".into(),
        },
        serde_json::json!({}),
    );
    store.append(ev).unwrap();

    // Nothing may leak into the artifact repo — its dir stays empty.
    let repo_entries = std::fs::read_dir(&ws.repo).unwrap().count();
    assert_eq!(
        repo_entries, 0,
        "event store must not pollute the artifact repo"
    );
}

#[test]
fn ensure_repo_initializes_a_git_repo_when_missing() {
    let (retain, repo, state) = sibling_dirs();
    let _ = retain;
    let ws = Workspace::open(&repo, &state, Selfhost::Disabled).unwrap();

    // No .git yet.
    assert!(!repo.join(".git").exists());

    // ensure_repo creates it.
    let created = ws.ensure_repo().unwrap();
    assert!(created, "should report it created the repo");
    assert!(repo.join(".git").exists(), ".git should now exist");

    // Idempotent: a second call reports it already existed.
    let again = ws.ensure_repo().unwrap();
    assert!(!again, "should report the repo already existed");
}

#[test]
fn ensure_repo_leaves_existing_repo_untouched() {
    let (retain, repo, state) = sibling_dirs();
    let _ = retain;
    let ws = Workspace::open(&repo, &state, Selfhost::Disabled).unwrap();

    // Pre-create a git repo with a commit.
    ws.git_command().arg("init").output().unwrap();
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    ws.git_command()
        .arg("add")
        .arg("README.md")
        .output()
        .unwrap();
    ws.git_command()
        .arg("commit")
        .arg("-m")
        .arg("initial")
        .output()
        .unwrap();
    let head_before = ws.head().unwrap();

    // ensure_repo should leave it alone.
    let created = ws.ensure_repo().unwrap();
    assert!(!created, "existing repo should not be re-initialized");
    assert_eq!(ws.head().unwrap(), head_before, "HEAD must not change");
}

#[test]
fn head_and_branch_resolve_after_init() {
    let (retain, repo, state) = sibling_dirs();
    let _ = retain;
    let ws = Workspace::open(&repo, &state, Selfhost::Disabled).unwrap();
    ws.ensure_repo().unwrap();

    // A fresh repo has no commits: head() is None, branch() is None.
    assert!(ws.head().is_none());
    assert!(ws.current_branch().is_none());

    // Make a commit, then HEAD and branch should resolve.
    std::fs::write(repo.join("file.txt"), "content\n").unwrap();
    ws.git_command().arg("add").arg(".").output().unwrap();
    ws.git_command()
        .arg("commit")
        .arg("-m")
        .arg("first")
        .output()
        .unwrap();
    assert!(ws.head().is_some(), "HEAD should resolve after a commit");
    assert!(
        ws.current_branch().is_some(),
        "branch should resolve after a commit"
    );
}
