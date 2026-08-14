# Telegram owner-messaging channel (2026-08-14)

Status: PLAN → IMPLEMENTED
Decision: **Telegram only** (WhatsApp/WeChat deferred — see below).

## Why Telegram, not WhatsApp/WeChat

The three platforms have wildly different setup burdens for a self-hosted,
local-first, single-project tool (`cast run <dir>` on a user's box, behind NAT):

- **Telegram** — free, no verification, no approval. Create a bot via @BotFather
  (2 min), user DMs it once. Works from anywhere.
- **WhatsApp** — official Cloud API needs a Meta-verified *business* + template
  pre-approval + per-message cost (wrong shape for a solo tool). Unofficial
  bridges (whatsapp-web.js etc.) are ToS-gray (ban risk) + a gateway dependency.
- **WeChat** — only *verified official/business accounts* get an API, which
  requires a mainland-China (or HK) registered entity; personal accounts have no
  sanctioned API. Effectively a non-starter outside the mainland-China market.

So: add a generic `Channel` seam now, ship **Telegram** as the reference adapter.

## Architecture

- **No webhook.** `getUpdates` long-polling instead — works behind NAT/local-first,
  needs no public HTTPS callback URL. Every message the bot re-polls; Telegram
  holds the response for `timeout` seconds (long poll).
- **Inbound** ≈ free: an owner Telegram message is appended as the *same*
  `MessageSent` event (`Actor::Owner`) that `POST /api/message` produces today.
  The PM loop already wakes on that (Tier-0 broadcast). No new plumbing.
- **Outbound**: the Telegram adapter is a **consumer** with its own durable
  cursor (`telegram`) — just like the reconciler/PM. Each drain it reads events
  since its cursor, and any `MessageSent` addressed `to:"owner"` is pushed to the
  owner's chat via `sendMessage`. Duplicated/superseded/ask decisions can be
  added later; v1 pushes owner-bound messages.

## `Channel` seam (`src/channel.rs`)

```rust
pub trait OwnerChannel: Send + Sync + 'static {
    /// Push a message to the owner (best-effort; the event log stays truth).
    fn notify(&self, text: &str) -> Result<()>;
}
```
- `NoopChannel` — default, does nothing (off unless configured).
- `TelegramChannel` — the reference adapter.

Making it a trait (not a concrete Telegram struct welded in) is the whole point:
WhatsApp/WeChat become a *new file + config*, never core changes.

## TelegramChannel (`src/telegram.rs`)

Env-gated, mirrors `CAST_LLM_*` (off by default, no cost, no network):

- `CAST_TELEGRAM_TOKEN` (required to enable)
- `CAST_TELEGRAM_CHAT_ID` (optional; if absent, learned from the first inbound
  owner message / or `getMe`)
- `CAST_TELEGRAM_POLL_SECS` (optional, default 30s)

Outbound: `POST https://api.telegram.org/bot<token>/sendMessage {chat_id, text}`.
Inbound: `POST .../getUpdates {offset, timeout}` loop, appending each owner text
message as `MessageSent` (deduped via update_id / a `telegram_in` cursor).

## Integration in `cast run` (`src/main.rs`)

Spawn a background `telegram::run(state, cfg)` tokio task when
`CAST_TELEGRAM_TOKEN` is set (mirroring `tokio::spawn(pm::run_pm(...))`).
Needs `AppState`'s broadcast tx so an appended inbound event wakes the PM.

## Tests (`tests/channel_telegram.rs`)

Local stub Telegram HTTP server on `127.0.0.1:0` (like `tests/llm_e2e.rs`):
- outbound `notify` POSTs `sendMessage` with correct bot token + chat_id + text
- inbound poll appends a `MessageSent` event the PM can see
- dedup: same update_id not double-appended
- `NoopChannel` touch is a no-op

## Deferred (explicitly NOT now)

WhatsApp/WeChat adapters; decision-ask push (needs "what needs the owner"
derivation wired to channel, a later slice); media/callbacks; encryption.