//! Telegram configure/status route logic (docs/plans/2026-08-14_telegram-channel.md).
//!
//! Tests the one-shot UI configure flow against a LOCAL stub Telegram server
//! (getMe + setMyName/setMyDescription + getUpdates) and the merge-based
//! config persistence — no live bot, no network, CI-safe.

use casting::runtime::telegram;
use casting::workspace::setup::{persist_telegram_config, read_config, RuntimeConfig};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const TOKEN: &str = "configure-test-token";
const PM_NAME: &str = "Sarah Chen";

/// Stub server: records branding calls + returns a seeded getUpdates payload.
struct StubState {
    branded: std::sync::Mutex<Vec<String>>,
    updates: std::sync::Mutex<Vec<serde_json::Value>>,
    get_me_calls: AtomicUsize,
}

impl StubState {
    fn new(updates: Vec<serde_json::Value>) -> Self {
        StubState {
            branded: std::sync::Mutex::new(vec![]),
            updates: std::sync::Mutex::new(updates),
            get_me_calls: AtomicUsize::new(0),
        }
    }
}

/// The stub Telegram API base URL. Serves:
///   GET/POST /bot<TOKEN>/getMe, /setMyName, /setMyDescription, /getUpdates
async fn boot_stub(stub: Arc<StubState>) -> String {
    use axum::routing::get;
    use axum::{routing::post, Json, Router};

    let app = Router::new()
        .route(
            &format!("/bot{TOKEN}/getMe"),
            get({
                let stub = stub.clone();
                move || {
                    let stub = stub.clone();
                    async move {
                        stub.get_me_calls.fetch_add(1, Ordering::SeqCst);
                        (
                            axum::http::StatusCode::OK,
                            Json(json!({
                                "ok": true,
                                "result": {
                                    "id": 9901, "is_bot": true,
                                    "first_name": "Old Bot Name",
                                    "username": "some_bot"
                                }
                            })),
                        )
                    }
                }
            }),
        )
        .route(
            &format!("/bot{TOKEN}/setMyName"),
            post({
                let stub = stub.clone();
                move |b: Json<serde_json::Value>| {
                    let stub = stub.clone();
                    async move {
                        stub.branded
                            .lock()
                            .unwrap()
                            .push(format!("name={}", b["name"].as_str().unwrap_or("")));
                        (
                            axum::http::StatusCode::OK,
                            Json(json!({"ok": true, "result": true})),
                        )
                    }
                }
            }),
        )
        .route(
            &format!("/bot{TOKEN}/setMyDescription"),
            post({
                let stub = stub.clone();
                move |b: Json<serde_json::Value>| {
                    let stub = stub.clone();
                    async move {
                        stub.branded
                            .lock()
                            .unwrap()
                            .push(format!("desc={}", b["description"].as_str().unwrap_or("")));
                        (
                            axum::http::StatusCode::OK,
                            Json(json!({"ok": true, "result": true})),
                        )
                    }
                }
            }),
        )
        .route(
            &format!("/bot{TOKEN}/getUpdates"),
            post({
                let stub = stub.clone();
                move |_b: Json<serde_json::Value>| {
                    let stub = stub.clone();
                    async move {
                        let results = stub.updates.lock().unwrap().clone();
                        (
                            axum::http::StatusCode::OK,
                            Json(json!({"ok": true, "result": results})),
                        )
                    }
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// configure validates the token, brands the bot as the PM, and learns the
/// chat_id from the first private-chat message.
#[tokio::test]
async fn configure_validates_brands_and_learns_chat() {
    let stub = Arc::new(StubState::new(vec![json!({
        "update_id": 1,
        "message": {"chat": {"id": 5555}, "text": "/start"}
    })]));
    let base = boot_stub(stub.clone()).await;

    let out = telegram::configure(TOKEN, PM_NAME, "desc", &base)
        .await
        .unwrap();

    assert_eq!(out.bot_id, 9901);
    assert_eq!(out.bot_name, "Old Bot Name"); // getMe returns the real name pre-brand
    assert_eq!(out.bot_username, "some_bot");
    assert_eq!(out.chat_id, Some(5555), "chat_id learned from getUpdates");
    assert_eq!(
        stub.get_me_calls.load(Ordering::SeqCst),
        1,
        "token validated via getMe"
    );
}

/// configure brands the bot's display name + description as the PM.
#[tokio::test]
async fn configure_brands_bot_as_pm() {
    let stub = Arc::new(StubState::new(vec![])); // no chat linked yet
    let base = boot_stub(stub.clone()).await;

    let out = telegram::configure(TOKEN, PM_NAME, "Your Casting PM.", &base)
        .await
        .unwrap();
    assert_eq!(
        out.chat_id, None,
        "no chat linked yet (owner hasn't DM'd the bot)"
    );

    let branded = stub.branded.lock().unwrap().clone();
    assert!(
        branded.iter().any(|s| s == "name=Sarah Chen"),
        "branded name: {branded:?}"
    );
    assert!(
        branded.iter().any(|s| s.contains("Casting PM")),
        "branded desc: {branded:?}"
    );
}

/// configure rejects an invalid token (getMe HTTP error).
#[tokio::test]
async fn configure_rejects_invalid_token() {
    use axum::{routing::get, Json, Router};
    let app = Router::new().route(
        &format!("/bot{TOKEN}/getMe"),
        get(|| async {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(json!({"ok": false})),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let err = telegram::configure(TOKEN, PM_NAME, "d", &format!("http://{addr}"))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("HTTP 401"),
        "surfaces the token rejection"
    );
}

/// persist_telegram_config merges into an existing config (never wipes the
/// director token), and read_config returns the Telegram fields.
#[test]
fn persist_merges_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    // Seed an existing config with a director token (as `cast init` would).
    std::fs::write(
        dir.path().join("config.json"),
        r#"{"name":"Acme Inc","director_token":"sekret"}"#,
    )
    .unwrap();

    persist_telegram_config(dir.path(), "token-123", 777).unwrap();

    let cfg: RuntimeConfig = read_config(dir.path()).unwrap();
    assert_eq!(cfg.name, "Acme Inc");
    assert_eq!(
        cfg.director_token.as_deref(),
        Some("sekret"),
        "director token preserved"
    );
    assert_eq!(cfg.telegram_token.as_deref(), Some("token-123"));
    assert_eq!(cfg.telegram_chat_id, Some(777));
}
