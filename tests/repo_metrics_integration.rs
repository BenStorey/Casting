//! Repo-metrics tests: reducer folds `RepoMetricsCaptured`, `capture()`
//! counts tracked files, and the git observer captures a snapshot when a PR
//! (merge) lands. Parser unit tests live in `src/repo_metrics.rs`.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::projection::Projection;
use casting::store::EventStore;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
use casting::workspace::repo_metrics;
/// One-shot env var set so the git observer debounce doesn't fire during tests.
fn _init_debounce() {
    std::env::set_var("CAST_GIT_DEBOUNCE_MS", "0");
}

use casting::workspace::{Selfhost, Workspace};

/// A fresh workspace with a real (empty) git repo.
fn ws_with_repo() -> (
    tempfile::TempDir,
    Workspace,
    SqliteEventStore,
    SqliteCursorStore,
) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let ws = Workspace::open(&repo, Selfhost::Disabled).unwrap();
    ws.ensure_repo().unwrap();
    let store = SqliteEventStore::open(ws.state_dir.join("events.db")).unwrap();
    let cursors = SqliteCursorStore::open(ws.state_dir.join("cursors.db")).unwrap();
    (tmp, ws, store, cursors)
}

/// Commit a 1-line Rust file so tokei/counting has something real to see.
/// Relies on ambient git identity (the CI setup sets a global one; local uses
/// Ben's). Do NOT use `git commit -c <cfg>` — `-c` is reedit-message, not config.
fn commit_rs(ws: &Workspace, name: &str, body: &str) {
    std::fs::write(ws.repo.join(name), body).unwrap();
    ws.git_command().arg("add").arg(name).output().unwrap();
    let out = ws
        .git_command()
        .args(["commit", "-m", &format!("add {name}")])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "commit should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn reducer_folds_repo_metrics_captured() {
    _init_debounce();
    let (retain, _ws, store, _cursors) = ws_with_repo();
    let _ = retain;

    let rm = casting::types::RepoMetrics {
        merge_sha: Some("abc123".into()),
        captured_at: "now".into(),
        file_count: 3,
        lines_by_language: vec![casting::types::LanguageLines {
            language: "Rust".into(),
            code: 10,
            comments: 1,
            blanks: 2,
            files: 1,
        }],
        coverage: Some(casting::types::CoverageInfo {
            percent: Some(50.0),
            source: "lcov.info".into(),
        }),
    };
    let ev = Event::new(
        "proj",
        Actor::System,
        EventType::RepoMetricsCaptured,
        Aggregate {
            kind: "repo-metrics".into(),
            id: "rm-x".into(),
        },
        serde_json::to_value(&rm).unwrap(),
    );
    store.append(ev).unwrap();

    let proj = Projection::build(&store, "proj").unwrap();
    assert_eq!(proj.repo_metrics.len(), 1);
    assert_eq!(proj.repo_metrics[0].file_count, 3);
    assert_eq!(proj.repo_metrics[0].lines_by_language[0].language, "Rust");
    assert_eq!(
        proj.repo_metrics[0].coverage.as_ref().unwrap().percent,
        Some(50.0)
    );
}

#[test]
fn capture_counts_tracked_files() {
    let (retain, ws, _store, _cursors) = ws_with_repo();
    let _ = retain;

    commit_rs(&ws, "main.rs", "fn main() {}\n");
    commit_rs(
        &ws,
        "lib.rs",
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    );

    let rm = repo_metrics::capture(&ws.repo);
    assert!(
        rm.file_count >= 1,
        "should count tracked files, got {}",
        rm.file_count
    );
    // tokei may not be on PATH in CI — only assert lines when present.
    if !rm.lines_by_language.is_empty() {
        let rust = rm
            .lines_by_language
            .iter()
            .any(|l| l.language.contains("Rust"));
        assert!(
            rust,
            "tokei should report some Rust, got: {:?}",
            rm.lines_by_language
        );
    }
}

#[tokio::test]
async fn observer_captures_repo_metrics_on_merge() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;

    commit_rs(&ws, "main.rs", "fn main() {}\n");
    let default_branch = ws.current_branch().unwrap();
    ws.git_command()
        .args(["checkout", "-b", "casting/task-7-feature"])
        .output()
        .unwrap();
    commit_rs(&ws, "feature.rs", "pub fn f() {}\n");
    ws.git_command()
        .args(["checkout", &default_branch])
        .output()
        .unwrap();
    ws.git_command()
        .args([
            "merge",
            "--no-ff",
            "casting/task-7-feature",
            "-m",
            "merge feature",
        ])
        .output()
        .unwrap();

    // Use observe_once (the async production path) so repo-metrics are
    // captured via AppState::append (integrity rail + broadcast).
    let state = casting::pm::AppState::new(store.clone(), cursors.clone(), "proj")
        .with_step_delay(std::time::Duration::ZERO);
    casting::workspace::git_observer::observe_once(&state, &ws).await;

    let proj = Projection::build(&store, "proj").unwrap();
    assert!(
        !proj.repo_metrics.is_empty(),
        "a merge was observed, so a repo-metrics snapshot should be captured"
    );
    let latest = proj
        .repo_metrics
        .iter()
        .find(|m| m.merge_sha.is_some())
        .expect("snapshot tagged with merge");
    assert!(latest.file_count > 0, "snapshot should count tracked files");
}
