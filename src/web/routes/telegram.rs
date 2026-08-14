//! Telegram owner-channel web routes (2026-08-14).
//!
//! `POST /api/telegram/configure` — a user of Casting pastes their BotFather
//! token; the server validates it, brands the bot as the PM (display name +
//! description), discovers the owner's chat_id, persists the secret to the
//! gitignored `.casting/config.json`, and starts the run loop. The result is
//! the bot identity + chat_id (or "chat not linked yet" if the owner hasn't
//! DM'd the bot).
//!
//! `GET /api/telegram/status` — read-only: is the channel configured + what's
//! the bot identity/chat (no secret returned).

use crate::pm::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct TelegramConfigureIn {
    token: String,
}

/// POST /api/telegram/configure — validate/paste-token, brand the PM bot,
/// learn the chat_id, persist, and start the loop.
pub(crate) async fn telegram_configure_handler(
    State(state): State<AppState>,
    Json(input): Json<TelegramConfigureIn>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let token = input.token.trim().to_string();
    if token.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "token is required".into()));
    }

    // The PM identity (name + description) the bot is branded with.
    let pm_name = "Sarah Chen";
    let pm_desc = "Your Casting Project Manager. Tell me what to build and I'll run the company.";

    let outcome = crate::telegram::configure(&token, pm_name, pm_desc, "")
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("telegram rejected token: {e:#}"),
            )
        })?;

    // Persist + start once the chat is linked. If not linked yet (owner hasn't
    // DM'd the bot), we still validate/brand; the loop waits for the DM.
    let started = match outcome.chat_id {
        Some(chat_id) => {
            if let Some(dir) = &state.state_dir {
                let _ = crate::setup::persist_telegram_config(dir, &token, chat_id);
            }
            crate::telegram::replace_loop(
                &state,
                crate::telegram::TelegramConfig::from_pieces(&token, chat_id),
            )
        }
        None => false,
    };

    Ok(Json(serde_json::json!({
        "bot_id": outcome.bot_id,
        "bot_name": outcome.bot_name,
        "bot_username": outcome.bot_username,
        "chat_id": outcome.chat_id,
        "chat_linked": outcome.chat_id.is_some(),
        "loop_started": started,
    })))
}

/// GET /api/telegram/status — is the channel configured, and with what bot /
/// chat (no secret). Read-only, not guarded.
pub(crate) async fn telegram_status_handler(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let configured = state
        .telegram_started
        .load(std::sync::atomic::Ordering::SeqCst);
    let (chat_id, name, username) = match &state.state_dir {
        Some(dir) => match crate::setup::read_config(dir) {
            Some(cfg) => (cfg.telegram_chat_id, None::<String>, None::<String>),
            None => (None, None, None),
        },
        None => (None, None, None),
    };

    Ok(Json(serde_json::json!({
        "configured": configured,
        "chat_id": chat_id,
        "bot_name": name,
        "bot_username": username,
    })))
}
