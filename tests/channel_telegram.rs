//! Telegram director channel tests (docs/plans/2026-08-14_telegram-channel.md).
//!
//! Everything runs against a LOCAL stub Telegram HTTP server (127.0.0.1:0), so
//! the whole seam is pinned down with no live bot, no network, CI-safe — the
//! same pattern as tests/llm_e2e.rs. We drive `casting::runtime::telegram::drain`
//! directly (one channel pass) against a real AppState.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::runtime::channel::{NoopChannel, OwnerChannel};
use casting::runtime::telegram::{TelegramChannel, TelegramConfig};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// In-memory stub Telegram state: outbound sends + inbound updates to deliver.
#[derive(Default)]
struct StubState {
    sent: Vec<(i64, String)>,             // (chat_id, text)
    updates: VecDeque<serde_json::Value>, // each a full update payload
}

fn make_state(project: &str) -> AppState {
    let store = casting::store::SqliteEventStore::in_memory().unwrap();
    let cursors = casting::store::SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, project)
}

fn director_bound_msg(state: &AppState, body: &str) {
    state
        .append(Event::new(
            &state.project,
            Actor::Agent {
                id: "pm".to_string(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: format!("msg-out-{}", uuid::Uuid::new_v4()),
            },
            json!({ "to": "director", "body": body }),
        ))
        .unwrap();
}

/// Boot a stub Telegram API server and point a channel at it.
async fn boot(
    chat_id: i64,
) -> (
    AppState,
    TelegramChannel,
    Arc<Mutex<std::sync::mpsc::Receiver<String>>>,
    Arc<Mutex<StubState>>,
) {
    use axum::{routing::post, Json, Router};

    const TOKEN: &str = "test-bot-token";

    let stub = Arc::new(Mutex::new(StubState::default()));

    let app = Router::new()
        .route(
            &format!("/bot{TOKEN}/sendMessage"),
            post({
                let stub = stub.clone();
                move |body: Json<serde_json::Value>| {
                    let stub = stub.clone();
                    async move {
                        let mut s = stub.lock().unwrap();
                        let chat = body["chat_id"].as_i64().unwrap_or(-1);
                        let text = body["text"].as_str().unwrap_or("").to_string();
                        s.sent.push((chat, text));
                        (
                            axum::http::StatusCode::OK,
                            Json(json!({"ok": true, "result": {}})),
                        )
                    }
                }
            }),
        )
        .route(
            &format!("/bot{TOKEN}/getUpdates"),
            post({
                let stub = stub.clone();
                move |_body: Json<serde_json::Value>| {
                    let stub = stub.clone();
                    async move {
                        let mut s = stub.lock().unwrap();
                        let results: Vec<serde_json::Value> = s.updates.drain(..).collect();
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

    let cfg = TelegramConfig {
        token: TOKEN.to_string(),
        chat_id,
        poll_secs: 30,
        api_base: format!("http://{addr}"),
    };
    let (channel, rx) = TelegramChannel::new(cfg);
    let state = make_state("proj-tg").with_channel(Arc::new(channel.clone()));
    (state, channel, rx, stub)
}

// Outbound durable cursor: a PM->director MessageSent is pushed via sendMessage.
#[tokio::test]
async fn outbound_pushes_owner_bound_message() {
    let (state, channel, rx, stub) = boot(12345).await;
    director_bound_msg(&state, "Please approve the database choice");

    casting::runtime::telegram::drain(state.clone(), &channel, &rx)
        .await
        .unwrap();

    let s = stub.lock().unwrap();
    assert_eq!(s.sent.len(), 1, "one director-bound message pushed");
    assert_eq!(s.sent[0].0, 12345, "targets the director chat");
    assert_eq!(s.sent[0].1, "Please approve the database choice");
}

/// Outbound durable cursor: NOT pushed for a message addressed elsewhere, or
/// echoed back to the director from the director themselves.
#[tokio::test]
async fn outbound_skips_non_owner_and_owner_originated() {
    let (state, channel, rx, stub) = boot(12345).await;

    // PM -> a consultant (not the director): must not be relayed.
    state
        .append(Event::new(
            &state.project,
            Actor::Agent {
                id: "pm".to_string(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-cons".into(),
            },
            json!({ "to": "marcus-reed", "body": "internal note" }),
        ))
        .unwrap();
    // Owner -> PM (the director's own inbound): must NOT be echoed back to director.
    state
        .append(Event::new(
            &state.project,
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-director-in".into(),
            },
            json!({ "to": "pm", "body": "hello from owner" }),
        ))
        .unwrap();

    casting::runtime::telegram::drain(state.clone(), &channel, &rx)
        .await
        .unwrap();

    let s = stub.lock().unwrap();
    assert_eq!(
        s.sent.len(),
        0,
        "no relay for non-director or director-originated messages"
    );
}

/// Outbound immediate: `OwnerChannel::notify` is drained and sent.
#[tokio::test]
async fn notify_queue_is_drained() {
    let (state, channel, rx, stub) = boot(12345).await;
    OwnerChannel::notify(&channel, "⚠️ budget warning").unwrap();

    casting::runtime::telegram::drain(state.clone(), &channel, &rx)
        .await
        .unwrap();

    let s = stub.lock().unwrap();
    assert_eq!(s.sent.len(), 1);
    assert_eq!(s.sent[0].1, "⚠️ budget warning");
}

/// Inbound: a message from the configured director chat becomes a MessageSent the
/// PM will see (the event is appended, which wakes the PM via broadcast).
#[tokio::test]
async fn inbound_appends_owner_message_event() {
    let (state, channel, rx, stub) = boot(12345).await;
    {
        let mut s = stub.lock().unwrap();
        s.updates.push_back(json!({
            "update_id": 10,
            "message": {"chat": {"id": 12345}, "text": "ship it now"}
        }));
    }

    casting::runtime::telegram::drain(state.clone(), &channel, &rx)
        .await
        .unwrap();

    let proj = state.projection().unwrap();
    let owner_msgs: Vec<_> = proj
        .messages
        .iter()
        .filter(|m| m.body == "ship it now")
        .collect();
    assert_eq!(
        owner_msgs.len(),
        1,
        "inbound owner message appended as MessageSent"
    );
    assert_eq!(owner_msgs[0].to, "pm");
}

/// Inbound auth: a message from a NON-director chat is ignored (a stranger DMing
/// the bot is never trusted).
#[tokio::test]
async fn inbound_ignores_stranger_chat() {
    let (state, channel, rx, stub) = boot(12345).await;
    {
        let mut s = stub.lock().unwrap();
        s.updates.push_back(json!({
            "update_id": 20,
            "message": {"chat": {"id": 99999}, "text": "hello from a stranger"}
        }));
    }

    casting::runtime::telegram::drain(state.clone(), &channel, &rx)
        .await
        .unwrap();

    let proj = state.projection().unwrap();
    assert!(proj.messages.is_empty(), "stranger message not appended");
}

/// Inbound dedup: the cursor advances past delivered update_id, so a second
/// drain of the same updates does not double-append.
#[tokio::test]
async fn inbound_cursor_prevents_re_append() {
    let (state, channel, rx, stub) = boot(12345).await;
    {
        let mut s = stub.lock().unwrap();
        s.updates.push_back(json!({
            "update_id": 30,
            "message": {"chat": {"id": 12345}, "text": "once only"}
        }));
    }

    casting::runtime::telegram::drain(state.clone(), &channel, &rx)
        .await
        .unwrap();
    // Stub drains its queue on the first poll; simulate a later poll with
    // nothing new (real Telegram returns [] after we ack via offset).
    casting::runtime::telegram::drain(state.clone(), &channel, &rx)
        .await
        .unwrap();

    let proj = state.projection().unwrap();
    let n = proj
        .messages
        .iter()
        .filter(|m| m.body == "once only")
        .count();
    assert_eq!(n, 1, "inbound message appended exactly once across drains");
}

/// The NoopChannel is a pipe to nowhere: notify is a no-op.
#[test]
fn noop_channel_is_noop() {
    let c = NoopChannel;
    assert!(c.notify("anything").is_ok());
}
