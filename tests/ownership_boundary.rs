//! Tests for the ownership boundary (docs/DIRECTORSHIP_BOUNDARY.md, D5): the
//! self-identity guard, the EXTERNAL state dir (state lives under a separate
//! dir, NEVER inside the artifact repo), path sandboxing, and the pinned git
//! runner. These encode the safety invariant that Casting can never conduct on
//! the wrong repo — least of all the repo that built it.
//!
//! All git-like tests use throwaway repos under tempdir; none ever touch the
//! product repo at /home/ben/casting.

use casting::store::EventStore;
use casting::store::SqliteEventStore;
use casting::workspace::{Selfhost, Workspace};
use std::path::Path;

/// A fresh, existing `repo` dir inside a tempdir, plus a SEPARATE state dir
/// (state lives OUTSIDE the repo, under `<tmp>/state/`).
fn repo_dir() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    (tmp, repo, state_dir)
}

#[test]
fn state_dir_is_outside_the_repo_and_resolved() {
    let (tmp, repo, state_dir) = repo_dir();
    let _ = tmp;

    let ws = Workspace::open(&repo, &state_dir, Selfhost::Disabled).unwrap();

    // State lives OUTSIDE the repo (never collocated in <repo>/.casting/), so the
    // artifact repo is never mutated by Casting's own data.
    assert_eq!(ws.casting_dir(), &state_dir);
    assert!(!ws.casting_dir().starts_with(&ws.repo));
    assert!(!ws.repo.starts_with(ws.casting_dir()));
}

#[test]
fn refuses_state_dir_inside_the_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    // Trying to put state INSIDE the repo must be refused.
    let err = Workspace::open(&repo, &repo.join(".casting"), Selfhost::Disabled)
        .expect_err("state dir inside the repo must be refused");
    assert!(
        err.to_string().contains("outside"),
        "error must explain state must live outside the repo: {err}"
    );
}

#[test]
fn refuses_the_repo_that_built_it_without_selfhost() {
    let root = option_env!("CASTING_SOURCE_ROOT");
    let Some(root) = root else {
        eprintln!("CASTING_SOURCE_ROOT not set at build time; skipping");
        return;
    };
    // Selfhost::Disabled must refuse the Casting source repo.
    let err = Workspace::open(
        Path::new(root),
        Path::new("/tmp/casting-state-1"),
        Selfhost::Disabled,
    )
    .expect_err("source repo should be refused without Selfhost::Enabled");
    assert!(
        err.to_string().contains("self-host"),
        "error must mention self-hosting: {err}"
    );
    // Selfhost::Enabled works trivially.
    Workspace::open(
        Path::new(root),
        Path::new("/tmp/casting-state-2"),
        Selfhost::Enabled,
    )
    .expect("source repo with Selfhost::Enabled should work");
}

#[test]
fn refuses_a_repo_with_casting_identity_without_selfhost() {
    let (retain, repo, state_dir) = repo_dir();
    let _ = retain;
    std::fs::write(repo.join("Cargo.toml"), "name = \"casting\"\n").unwrap();
    // Selfhost::Disabled must refuse a repo with Casting identity.
    let err = Workspace::open(&repo, &state_dir, Selfhost::Disabled)
        .expect_err("a repo naming Casting should be refused without Selfhost::Enabled");
    assert!(
        err.to_string().contains("self-host"),
        "error must mention self-hosting: {err}"
    );
    // Selfhost::Enabled allows it.
    let ws = Workspace::open(&repo, &state_dir, Selfhost::Enabled).unwrap();
    assert_eq!(ws.selfhost(), Selfhost::Enabled);
}

#[test]
fn resolve_under_resolves_inside_repo_and_rejects_escape() {
    let (retain, repo, state_dir) = repo_dir();
    let _ = retain;
    let ws = Workspace::open(&repo, &state_dir, Selfhost::Disabled).unwrap();

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
    let (retain, repo, state_dir) = repo_dir();
    let _ = retain;
    let ws = Workspace::open(&repo, &state_dir, Selfhost::Disabled).unwrap();

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
fn event_store_lives_in_external_state_dir_and_repo_stays_clean() {
    let (retain, repo, state_dir) = repo_dir();
    let _ = retain;
    let ws = Workspace::open(&repo, &state_dir, Selfhost::Disabled).unwrap();
    ws.ensure_repo().unwrap();

    // Open the store via the EXTERNAL state dir and append a real event.
    let store = SqliteEventStore::open(ws.casting_dir().join("events.db")).unwrap();
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

    // The state DB lives in the EXTERNAL state dir, and the repo is untouched.
    assert!(ws.casting_dir().join("events.db").exists());
    assert!(!repo.join("events.db").exists());
    assert!(
        !repo.join(".casting").exists(),
        "no .casting must appear in the repo"
    );
    // The repo has nothing tracked beyond what we did not add.
    ws.git_command().arg("add").arg("-A").output().unwrap();
    let status = ws
        .git_command()
        .arg("status")
        .arg("--porcelain")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&status.stdout).to_string();
    assert!(
        !out.contains("events.db"),
        "events.db must not land in the repo: {out}"
    );
}

#[test]
fn ensure_repo_initializes_a_git_repo_when_missing() {
    let (retain, repo, state_dir) = repo_dir();
    let _ = retain;
    let ws = Workspace::open(&repo, &state_dir, Selfhost::Disabled).unwrap();

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
    let (retain, repo, state_dir) = repo_dir();
    let _ = retain;
    let ws = Workspace::open(&repo, &state_dir, Selfhost::Disabled).unwrap();

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
    let (retain, repo, state_dir) = repo_dir();
    let _ = retain;
    let ws = Workspace::open(&repo, &state_dir, Selfhost::Disabled).unwrap();
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
