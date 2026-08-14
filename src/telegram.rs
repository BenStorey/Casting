//! Telegram owner channel (2026-08-14, docs/plans/2026-08-14_telegram-channel.md).
//!
//! The reference adapter for the [`crate::channel::OwnerChannel`] seam. Runs as
//! an INDEPENDENT cursor-driven consumer (like the reconciler / watchdog — never
//! welded into the PM), so the event log stays the only authority and the
//! channel is a best-effort mirror:
//!
//! - **Outbound:** since its durable `telegram:out` cursor, every new
//!   `MessageSent` event addressed `to:"owner"` (from a non-owner actor) is
//!   pushed to the owner's chat via `sendMessage`. The immediate `notify` queue
//!   is drained in the same pass (for one-off push e.g. guard alerts).
//! - **Inbound:** long-poll `getUpdates`; each owner message becomes the SAME
//!   `MessageSent` event (`Actor::Owner`) that `POST /api/message` produces, so
//!   the PM wakes on it for free (Tier-0 broadcast) — zero new plumbing.
//!
//! Long-polling (`getUpdates`) rather than a webhook on purpose: works behind
//! NAT / local-first, needs no public HTTPS callback, fits `cast run <dir>`.
//!
//! Env-gated, mirrors the LLM seam (off by default, no network, no cost):
//! `CAST_TELEGRAM_TOKEN` (required), `CAST_TELEGRAM_CHAT_ID` (required for v1 —
//! binds "who is the owner" so a stranger DMing the bot is never treated as the
//! owner; also the sendMessage target), `CAST_TELEGRAM_POLL_SECS` (default 30).

use crate::channel::OwnerChannel;
use crate::cursor::CursorStore;
use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use anyhow::Result;
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Durable consumer ids for this channel's outbound/inbound cursors.
const OUT_CURSOR: &str = "telegram:out";
const IN_CURSOR: &str = "telegram:in";

/// Configuration for the Telegram channel.
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    pub token: String,
    pub chat_id: i64,
    pub poll_secs: u64,
    /// API base for tests to point at a stub; production = the real endpoint.
    pub api_base: String,
}

impl TelegramConfig {
    /// Read `CAST_TELEGRAM_TOKEN` + `CAST_TELEGRAM_CHAT_ID` (+ optional
    /// `CAST_TELEGRAM_POLL_SECS`). Returns `None` when unconfigured (the
    /// channel stays off, no network/cost — mirrors the LLM seam).
    pub fn from_env() -> Option<Self> {
        let token = std::env::var("CAST_TELEGRAM_TOKEN").ok()?;
        if token.trim().is_empty() {
            return None;
        }
        let chat_id = std::env::var("CAST_TELEGRAM_CHAT_ID").ok()?.parse().ok()?;
        let poll_secs = std::env::var("CAST_TELEGRAM_POLL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        Some(TelegramConfig {
            token,
            chat_id,
            poll_secs,
            api_base: "https://api.telegram.org".to_string(),
        })
    }
}

/// The Telegram `OwnerChannel`. `notify` enqueues to an outbox the run loop
/// drains (non-blocking, safe to call from sync code); the durable cursor
/// path is the primary outbound, the outbox is the immediate one-off path.
#[derive(Clone)]
pub struct TelegramChannel {
    config: Arc<TelegramConfig>,
    http: reqwest::Client,
    outbox: std::sync::mpsc::Sender<String>,
}

impl TelegramChannel {
    /// A channel plus the receiving side of its outbox queue.
    pub fn new(config: TelegramConfig) -> (Self, Arc<Mutex<std::sync::mpsc::Receiver<String>>>) {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("build reqwest client");
        (
            TelegramChannel {
                config: Arc::new(config),
                http,
                outbox: tx,
            },
            Arc::new(Mutex::new(rx)),
        )
    }

    fn url(&self, method: &str) -> String {
        format!(
            "{}/bot{}/{}",
            self.config.api_base.trim_end_matches('/'),
            self.config.token,
            method
        )
    }

    /// `sendMessage` to the owner's chat. The durable cursor path is the real
    /// outbound; this is the raw primitive both paths share.
    async fn send_message(&self, text: &str) -> Result<()> {
        let resp = self
            .http
            .post(self.url("sendMessage"))
            .json(&serde_json::json!({
                "chat_id": self.config.chat_id,
                "text": text,
            }))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("telegram sendMessage returned HTTP {status}: {body}");
        }
        Ok(())
    }

    /// `getUpdates` long-poll (offset-based, idempotent: acknowledge by
    /// passing the highest seen update_id + 1 next call).
    async fn get_updates(&self, offset: u64, timeout: u64) -> Result<Vec<Update>> {
        let resp = self
            .http
            .post(self.url("getUpdates"))
            .json(&serde_json::json!({
                "offset": offset,
                "timeout": timeout,
                "allowed_updates": ["message"],
            }))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("telegram getUpdates returned HTTP {status}: {body}");
        }
        let payload: ApiPayload<Vec<Update>> = resp.json().await?;
        if !payload.ok {
            anyhow::bail!("telegram getUpdates denied: {:?}", payload.description);
        }
        Ok(payload.result)
    }
}

impl OwnerChannel for TelegramChannel {
    fn notify(&self, text: &str) -> Result<()> {
        // Best-effort enqueue; the run loop drains it. Errors just mean the
        // immediate push was dropped — the durable cursor path protects the
        // event log regardless.
        let _ = self.outbox.send(text.to_string());
        Ok(())
    }
}

/// One Telegram update (the message subset we care about).
#[derive(Debug, Deserialize)]
pub struct Update {
    pub update_id: u64,
    #[serde(default)]
    pub message: Option<UpdateMessage>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMessage {
    pub chat: UpdateChat,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChat {
    pub id: i64,
}

#[derive(Deserialize)]
struct ApiPayload<T> {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    result: T,
}

/// The durable cursor-backed run loop. Spawned by `cast run` when the channel
/// is configured; idempotent and resumable across restarts.
pub async fn run(
    state: AppState,
    channel: TelegramChannel,
    rx: Arc<Mutex<std::sync::mpsc::Receiver<String>>>,
) {
    let cfg = channel.config.clone();
    eprintln!(
        "📲 telegram channel enabled (chat_id={}, poll {}s)",
        cfg.chat_id, cfg.poll_secs
    );

    loop {
        match drain(state.clone(), &channel, &rx).await {
            Ok(()) => {}
            Err(e) => eprintln!("⚠️  telegram channel pass failed: {e:#}"),
        }
        tokio::time::sleep(Duration::from_secs(cfg.poll_secs)).await;
    }
}

/// One full channel pass: outbound (immediate queue + durable owner-message
/// cursor) then inbound (getUpdates → append owner MessageSent). Errors are
/// surfaced; the event log is never corrupted by a transport failure.
pub async fn drain(
    state: AppState,
    channel: &TelegramChannel,
    rx: &Arc<Mutex<std::sync::mpsc::Receiver<String>>>,
) -> Result<()> {
    // ---- Outbound 1: immediate notify queue ----
    loop {
        let text = {
            let guard = rx.lock().map_err(|_| anyhow::anyhow!("outbox poisoned"))?;
            guard.try_recv().ok()
        };
        match text {
            Some(t) => channel.send_message(&t).await?,
            None => break,
        }
    }

    // ---- Outbound 2: durable owner-message cursor ----
    let mut out = state.cursors.get(&state.project, OUT_CURSOR)?.last_seen;
    let latest = state.store.latest_sequence(&state.project)?;
    if latest > out {
        let events = state.store.read_since(&state.project, out)?;
        for ev in &events {
            // Push only NEW MessageSent events addressed to the owner, from a
            // non-owner (never echo the owner's own inbound message back).
            if ev.event_type == EventType::MessageSent
                && ev.actor != Actor::Owner
                && owner_bound(ev)
            {
                let body = string_field(ev, "body").unwrap_or_default();
                if !body.is_empty() && !body.starts_with("msg-") {
                    channel.send_message(&body).await?;
                }
            }
            out = out.max(ev.sequence); // keep cursor even if a send fails
        }
        state.cursors.advance(&state.project, OUT_CURSOR, out)?;
    }

    // ---- Inbound: getUpdates → append owner MessageSent ----
    let in_seq = state.cursors.get(&state.project, IN_CURSOR)?.last_seen as u64;
    // Long-poll: ask Telegram to hold the connection briefly (1s poll offset;
    // our own sleep keeps cadence). Tests set poll_secs low so a sender is
    // never left hanging on a long-held response.
    let updates = channel.get_updates(in_seq, 1).await?;
    let last_update = updates.iter().map(|u| u.update_id).max();
    for u in &updates {
        if let Some(msg) = &u.message {
            // Only the configured owner chat is treated as the owner. This is
            // the Telegram-side auth: a stranger DMing the bot is never trusted.
            if msg.chat.id == channel.config.chat_id {
                if let Some(text) = msg.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                    let ev = Event::new(
                        &state.project,
                        Actor::Owner,
                        EventType::MessageSent,
                        Aggregate {
                            kind: "message".into(),
                            id: format!("msg-{}", uuid::Uuid::new_v4()),
                        },
                        serde_json::json!({ "to": "pm", "body": text }),
                    );
                    // append broadcasts → wakes the PM (Tier-0) for free.
                    state.append(ev)?;
                }
            }
        }
    }
    if let Some(id) = last_update {
        state
            .cursors
            .advance(&state.project, IN_CURSOR, (id + 1) as i64)?;
    }
    Ok(())
}

/// Does this `MessageSent` event address the owner?
fn owner_bound(ev: &Event) -> bool {
    string_field(ev, "to").as_deref() == Some("owner")
}

fn string_field(ev: &Event, key: &str) -> Option<String> {
    ev.data
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
