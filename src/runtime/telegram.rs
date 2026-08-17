//! Telegram director channel (2026-08-14, docs/plans/2026-08-14_telegram-channel.md).
//!
//! The reference adapter for the [`crate::runtime::channel::OwnerChannel`] seam. Runs as
//! an INDEPENDENT cursor-driven consumer (like the reconciler / watchdog — never
//! welded into the PM), so the event log stays the only authority and the
//! channel is a best-effort mirror:
//!
//! - **Outbound:** since its durable `telegram:out` cursor, every new
//!   `MessageSent` event addressed `to:"director"` (from a non-director actor) is
//!   pushed to the director's chat via `sendMessage`. The immediate `notify` queue
//!   is drained in the same pass (for one-off push e.g. guard alerts).
//! - **Inbound:** long-poll `getUpdates`; each director message becomes the SAME
//!   `MessageSent` event (`Actor::Director { user_id: "ceo".into() }`) that `POST /api/message` produces, so
//!   the PM wakes on it for free (Tier-0 broadcast) — zero new plumbing.
//!
//! Long-polling (`getUpdates`) rather than a webhook on purpose: works behind
//! NAT / local-first, needs no public HTTPS callback, fits `cast run <dir>`.
//!
//! Env-gated, mirrors the LLM seam (off by default, no network, no cost):
//! `CAST_TELEGRAM_TOKEN` (required), `CAST_TELEGRAM_CHAT_ID` (required for v1 —
//! binds "who is the director" so a stranger DMing the bot is never treated as the
//! director; also the sendMessage target), `CAST_TELEGRAM_POLL_SECS` (default 30).

use crate::event::{Actor, Aggregate, Event, EventType};
use crate::pm::AppState;
use crate::runtime::channel::OwnerChannel;
use crate::store::CursorStore;
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
    /// Build a config from explicit pieces (token + chat_id), for callers that
    /// resolved them from persisted config / a UI configure flow rather than
    /// env. `poll_secs` defaults to 30.
    pub fn from_pieces(token: impl Into<String>, chat_id: i64) -> Self {
        TelegramConfig {
            token: token.into(),
            chat_id,
            poll_secs: 30,
            api_base: "https://api.telegram.org".to_string(),
        }
    }

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

    /// `sendMessage` to the director's chat. The durable cursor path is the real
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

// === Token-level operations (for the UI configure flow) ===================
//
// These work on a RAW bot token — validation, branding, chat_id discovery —
// independent of a fully-configured TelegramChannel. They back
// `POST /api/telegram/configure` so a user of Casting pastes a BotFather
// token and the server does the rest.

/// The bot identity returned by `getMe` (validates a token + shows what the
/// bot is called / its username).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BotIdentity {
    pub id: i64,
    #[serde(default)]
    pub first_name: String,
    #[serde(default, rename = "username")]
    pub username: String,
    #[serde(default = "default_true")]
    pub is_bot: bool,
}

fn default_true() -> bool {
    true
}

/// Validate a raw token + return the bot identity. Errors if the token is
/// rejected by Telegram. `api_base` "" = the real endpoint (overridable for
/// stub tests).
pub async fn get_me(token: &str, api_base: &str) -> Result<BotIdentity> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let base = if api_base.is_empty() {
        "https://api.telegram.org".to_string()
    } else {
        api_base.to_string()
    };
    let url = format!("{}/bot{token}/getMe", base.trim_end_matches('/'));
    let resp = http.get(&url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("invalid bot token (HTTP {status})"));
    }
    let payload: ApiPayload<BotIdentity> = resp.json().await?;
    if !payload.ok {
        return Err(anyhow::anyhow!(
            "telegram rejected token: {:?}",
            payload.description
        ));
    }
    Ok(payload.result)
}

/// Brand the bot as the director's PM: set its display name + short description.
/// This is what makes the bot *be* the PM in the user's chat list. Best-effort
/// — name/description branding failing should not block config persistence.
async fn brand_bot(token: &str, name: &str, description: &str, api_base: &str) -> Result<()> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let base = if api_base.is_empty() {
        "https://api.telegram.org".to_string()
    } else {
        api_base.to_string()
    };
    for (method, value) in [
        ("setMyName", serde_json::json!({ "name": name })),
        (
            "setMyDescription",
            serde_json::json!({ "description": description }),
        ),
    ] {
        let url = format!("{}/bot{token}/{method}", base.trim_end_matches('/'));
        let resp = http
            .post(&url)
            .json(&value)
            .send()
            .await?
            .error_for_status()?;
        let payload: ApiPayload<serde_json::Value> = resp.json().await?;
        if !payload.ok {
            anyhow::bail!("telegram {method} refused: {:?}", payload.description);
        }
    }
    Ok(())
}

/// Discover the director's chat_id: the FIRST private-chat message the bot has
/// received (`getUpdates`). Telegram requires the user to DM the bot once, so
/// this is the natural "link me" step — the user never types a chat_id.
/// `api_base` is overridable for tests (a stub server).
pub async fn discover_chat_id(token: &str, api_base: &str) -> Result<Option<i64>> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let base = if api_base.is_empty() {
        "https://api.telegram.org".to_string()
    } else {
        api_base.to_string()
    };
    let url = format!("{}/bot{token}/getUpdates", base.trim_end_matches('/'));
    let resp = http
        .post(&url)
        .json(&serde_json::json!({ "timeout": 2, "allowed_updates": ["message"] }))
        .send()
        .await?
        .error_for_status()?;
    let payload: ApiPayload<Vec<Update>> = resp.json().await?;
    if !payload.ok {
        anyhow::bail!("telegram getUpdates refused: {:?}", payload.description);
    }
    // First private-chat message from a non-bot is the director.
    for u in payload.result {
        if let Some(m) = u.message {
            if m.chat.id > 0 {
                // positive chat ids = private chats (not groups)
                return Ok(Some(m.chat.id));
            }
        }
    }
    Ok(None)
}

/// The result of a UI `configure` call: validated bot identity + the learned
/// chat_id (or None if the director hasn't DM'd the bot yet).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigureOutcome {
    pub bot_id: i64,
    pub bot_name: String,
    pub bot_username: String,
    pub chat_id: Option<i64>,
    /// Whether the run loop was started for the first time.
    pub loop_started: bool,
}

/// One-shot UI configure flow: validate the pasted BotFather token, brand the
/// bot as the PM (display name + description), discover the director's chat_id,
/// and (if found) persist + start the loop. `api_base` "" = real Telegram
/// (overridable for stub tests).
pub async fn configure(
    token: &str,
    pm_name: &str,
    pm_description: &str,
    api_base: &str,
) -> Result<ConfigureOutcome> {
    let me = get_me(token, api_base).await?;
    // Brand best-effort: a name/description-set failure shouldn't block config.
    let _ = brand_bot(token, pm_name, pm_description, api_base).await;
    let chat_id = discover_chat_id(token, api_base).await?;
    Ok(ConfigureOutcome {
        bot_id: me.id,
        bot_name: me.first_name,
        bot_username: me.username,
        chat_id,
        loop_started: false,
    })
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

struct ApiPayload<T> {
    ok: bool,
    description: Option<String>,
    result: T,
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for ApiPayload<T> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw<T> {
            ok: bool,
            #[serde(default)]
            description: Option<String>,
            result: T,
        }
        Raw::deserialize(deserializer).map(|r| ApiPayload {
            ok: r.ok,
            description: r.description,
            result: r.result,
        })
    }
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

/// Start the Telegram run loop from a config, exactly ONCE per AppState.
/// Composable: called from boot env wiring OR the UI configure route; the
/// `telegram_started` AtomicBool makes it idempotent. Must run inside a tokio
/// runtime. Returns true if this call started the loop (false = already on).
pub fn start_loop(state: &AppState, cfg: TelegramConfig) -> bool {
    use std::sync::atomic::Ordering;
    if state.telegram_started.swap(true, Ordering::SeqCst) {
        return false;
    }
    spawn_loop(state, cfg);
    true
}

/// Replace the running Telegram loop with a NEW one (new token / chat_id). Used
/// by `POST /api/telegram/configure` so a user can reconnect messaging any
/// time, not just at first-run/boot. Aborts the previous loop (if any), starts
/// a fresh cursor-driven loop, and returns true.
pub fn replace_loop(state: &AppState, cfg: TelegramConfig) -> bool {
    // Abort any previously-running loop.
    if let Some(handle) = state.telegram_handle.lock().unwrap().take() {
        handle.abort();
    }
    use std::sync::atomic::Ordering;
    state.telegram_started.store(true, Ordering::SeqCst);
    spawn_loop(state, cfg);
    true
}

/// Spawn the run loop + record its JoinHandle (assumes the caller already
/// decided this should happen — start_loop guards, replace_loop aborts first).
fn spawn_loop(state: &AppState, cfg: TelegramConfig) {
    let (channel, rx) = TelegramChannel::new(cfg);
    let state_with = state.clone().with_channel(Arc::new(channel.clone()));
    let handle = tokio::spawn(run(state_with, channel, rx));
    *state.telegram_handle.lock().unwrap() = Some(handle);
}

/// One full channel pass: outbound (immediate queue + durable director-message
/// cursor) then inbound (getUpdates → append director MessageSent). Errors are
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
            Some(t) => {
                // A transient send failure must NOT abort the drain: this is the
                // immediate outbox (best-effort "notify now"). Log and continue.
                if let Err(e) = channel.send_message(&t).await {
                    eprintln!("[telegram] outbox send failed: {e:#}");
                }
            }
            None => break,
        }
    }

    // ---- Outbound 2: durable director-message cursor ----
    let mut out = state.cursors.get(&state.project, OUT_CURSOR)?.last_seen;
    let latest = state.store.latest_sequence(&state.project)?;
    if latest > out {
        let events = state.store.read_since(&state.project, out)?;
        for ev in &events {
            // Push only NEW MessageSent events addressed to the director, from a
            // non-director (never echo the director's own inbound message back).
            if ev.event_type == EventType::MessageSent
                && ev.actor
                    != (Actor::Director {
                        user_id: "ceo".into(),
                    })
                && owner_bound(ev)
            {
                let body = string_field(ev, "body").unwrap_or_default();
                if !body.is_empty() && !body.starts_with("msg-") {
                    // A transient send failure must NOT abort the drain BEFORE
                    // the durable out-cursor advances (that re-sends the whole
                    // window next pass = duplicate pushes to the director). Log,
                    // keep going, and let the cursor advance past it so each
                    // message is sent at-most-once-per-cursor.
                    if let Err(e) = channel.send_message(&body).await {
                        eprintln!(
                            "[telegram] outbound send failed (cursor still advances to avoid dup): {e:#}"
                        );
                    }
                }
            }
            out = out.max(ev.sequence); // keep cursor even if a send fails
        }
        state.cursors.advance(&state.project, OUT_CURSOR, out)?;
    }

    // ---- Inbound: getUpdates → append director MessageSent ----
    let in_seq = state.cursors.get(&state.project, IN_CURSOR)?.last_seen as u64;
    // Long-poll: ask Telegram to hold the connection briefly (1s poll offset;
    // our own sleep keeps cadence). Tests set poll_secs low so a sender is
    // never left hanging on a long-held response.
    let updates = channel.get_updates(in_seq, 1).await?;
    let last_update = updates.iter().map(|u| u.update_id).max();
    for u in &updates {
        if let Some(msg) = &u.message {
            // Only the configured director chat is treated as the director. This is
            // the Telegram-side auth: a stranger DMing the bot is never trusted.
            if msg.chat.id == channel.config.chat_id {
                if let Some(text) = msg.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                    let ev = Event::new(
                        &state.project,
                        Actor::Director {
                            user_id: "ceo".into(),
                        },
                        EventType::MessageSent,
                        Aggregate {
                            kind: "message".into(),
                            id: format!("msg-{}", uuid::Uuid::new_v4()),
                        },
                        serde_json::json!({ "to": "mei", "body": text }),
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

/// Does this `MessageSent` event address the director?
fn owner_bound(ev: &Event) -> bool {
    string_field(ev, "to").as_deref() == Some("director")
}

fn string_field(ev: &Event, key: &str) -> Option<String> {
    ev.data
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}
