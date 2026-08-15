//! Owner-channel seam (2026-08-14, docs/plans/2026-08-14_telegram-channel.md).
//!
//! The owner already reaches the project via the web UI (`POST /api/message`)
//! and the PM reaches the owner via `MessageSent` events addressed `to:"owner"`
//! folded into `Projection.messages`. This seam is the *external transport* for
//! that conversation, so the owner can message the company from a phone.
//!
//! It is deliberately a trait — NOT a concrete Telegram struct welded into the
//! core. Telegram is the reference adapter (free, no verification, works behind
//! NAT); WhatsApp/WeChat are much heavier (business verification, entity
//! registration, cost) and are future *new files + config*, never core changes.
//! The event log / projection remain the only source of truth; a channel is a
//! best-effort mirror, never new state.

use anyhow::Result;

/// A best-effort outbound pipe to the owner. Never authoritative: a dropped
/// message must not corrupt the event log (the projection is the truth).
pub trait OwnerChannel: Send + Sync + 'static {
    /// Push a line of text to the owner. Errors are surfaced but must not
    /// abort a drain (best-effort transport).
    fn notify(&self, text: &str) -> Result<()>;
}

/// The default: does nothing. Off unless a real channel is configured, so
/// `cast run` has zero network/cost by default (mirrors the LLM seam).
#[derive(Debug, Clone, Default)]
pub struct NoopChannel;

impl OwnerChannel for NoopChannel {
    fn notify(&self, _text: &str) -> Result<()> {
        Ok(())
    }
}
