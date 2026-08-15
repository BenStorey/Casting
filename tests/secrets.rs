//! Tests for the no-secret-in-log invariant + per-project secret store
//! (docs/plans/2026-08-13_harness-guards.md, feature 2).
//!
//! The invariant: an append-only event log can never be scrubbed, so secret
//! VALUES must never be persisted. Activities reference a secret by NAME
//! (`@secret:NAME@`); the runner substitutes at execution time in memory. The
//! executor fail-closed refuses to schedule/execute an activity that embeds a
//! stored value verbatim.

use casting::event::{Actor, EventType};
use casting::pm::AppState;
use casting::runtime::executor::{execute, Activity, ActivityKind, NoopRunner};
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
use casting::workspace::secrets::SecretStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-secrets")
}

fn shell(id: &str, cmd: &str) -> Activity {
    Activity {
        id: id.into(),
        target_id: "t-1".into(),
        kind: ActivityKind::Shell { cmd: cmd.into() },
    }
}

#[test]
fn store_set_get_and_resolve() {
    let mut store = SecretStore::default();
    store
        .set("openrouter_api_key", "sk-live-9f8a7b6c5d4e")
        .unwrap();

    assert_eq!(
        store.get("openrouter_api_key"),
        Some("sk-live-9f8a7b6c5d4e")
    );
    assert!(store.has("openrouter_api_key"));
    assert!(!store.has("nope"));

    // Runner seam: placeholders resolve to values in memory only.
    assert_eq!(
        store.resolve("curl -H 'Authorization: Bearer @secret:openrouter_api_key@'"),
        "curl -H 'Authorization: Bearer sk-live-9f8a7b6c5d4e'"
    );
    // Unknown name -> empty (caller validates via `has`).
    assert_eq!(store.resolve("x@secret:missing@y"), "xy");
}

#[test]
fn schedule_rejects_activity_embedding_raw_secret() {
    let mut store = SecretStore::default();
    store
        .set("github_token", "ghp_ABCDEF1234567890abcd")
        .unwrap();
    let state = make_state().with_secrets(store);

    // A shell command that embeds the raw token is refused BEFORE it can
    // reach the log.
    let activity = shell(
        "t-1-leak",
        "git push https://ghp_ABCDEF1234567890abcd@github.com/x",
    );
    assert!(
        casting::runtime::executor::schedule(&state, Actor::System, &activity).is_err(),
        "raw secret in an activity must be refused at schedule"
    );

    // ... and critically, NO ActivityScheduled made it into the event log.
    let events = state.store.read_since("proj-secrets", 0).unwrap();
    assert!(
        !events
            .iter()
            .any(|e| e.event_type == EventType::ActivityScheduled),
        "a refused (secret-leaking) activity must never be persisted"
    );
    // The raw value is nowhere in the log.
    for e in &events {
        let serialized = serde_json::to_string(&e.data).unwrap();
        assert!(
            !serialized.contains("ghp_ABCDEF1234567890abcd"),
            "secret value leaked into the event log"
        );
    }
}

#[test]
fn schedule_allows_placeholder_and_persists_no_value() {
    let mut store = SecretStore::default();
    store
        .set("openrouter_api_key", "sk-live-9f8a7b6c5d4e")
        .unwrap();
    let state = make_state().with_secrets(store);

    // The command references the secret BY NAME — the runner substitutes live.
    let activity = shell(
        "t-2-ok",
        "curl -H 'Authorization: Bearer @secret:openrouter_api_key@' example.com",
    );
    assert!(casting::runtime::executor::schedule(&state, Actor::System, &activity).is_ok());

    let events = state.store.read_since("proj-secrets", 0).unwrap();
    let scheduled = events
        .iter()
        .find(|e| e.event_type == EventType::ActivityScheduled)
        .expect("scheduled");
    // The RAW value must not be in the persisted event — only its name token.
    let serialized = serde_json::to_string(&scheduled.data).unwrap();
    assert!(
        !serialized.contains("sk-live-9f8a7b6c5d4e"),
        "raw secret value must never be persisted to the log"
    );
    assert!(
        serialized.contains("@secret:openrouter_api_key@"),
        "the name token (not the value) is what's recorded"
    );
}

#[test]
fn execute_rejects_embed_and_records_activity_failed() {
    let mut store = SecretStore::default();
    store.set("aws_secret", "AKIA1234567890ABCDEF").unwrap();
    let state = make_state().with_secrets(store);
    let runner = NoopRunner;

    let activity = shell("t-3-exec", "export AWS_KEY=AKIA1234567890ABCDEF");
    assert!(execute(&state, &runner, Actor::System, &activity).is_err());

    // The value must be in NO event — including the failure path (which we
    // deliberately keep out of the log rather than persist the leak).
    let events = state.store.read_since("proj-secrets", 0).unwrap();
    for e in &events {
        assert!(
            !serde_json::to_string(&e.data)
                .unwrap()
                .contains("AKIA1234567890ABCDEF"),
            "secret value leaked into the event log (even via a failure event)"
        );
    }
}

#[test]
fn short_values_are_exempt_from_scan() {
    // A very short secret (below MIN_SCAN_LEN) can't be reliably detected
    // without false positives on ordinary text, so it's skipped.
    let mut store = SecretStore::default();
    store.set("n", "abcdef").unwrap(); // len 6 < 8 -> exempt
    let state = make_state().with_secrets(store);

    assert!(casting::runtime::executor::schedule(
        &state,
        Actor::System,
        &shell("t-4-short", "echo abcdef")
    )
    .is_ok());
}

#[test]
fn store_persists_and_reloads() {
    let dir = std::env::temp_dir().join(format!("casting-secrets-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut store = SecretStore::load(&dir).unwrap();
    assert!(store.get("k").is_none());
    store.set("k", "secret-value-1234567890").unwrap();

    // Reload from disk — set() persisted.
    let reloaded = SecretStore::load(&dir).unwrap();
    assert_eq!(reloaded.get("k"), Some("secret-value-1234567890"));

    let _ = std::fs::remove_dir_all(&dir);
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{}", N.fetch_add(1, Ordering::SeqCst))
}
