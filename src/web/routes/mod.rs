//! Axum routes for the Casting API, split by concern. Each submodule owns the
//! handlers + request DTOs for one slice of the surface; this module
//! reassembles them into the single application router served by the binary.

mod advisor;
mod auth;
mod inbox;
mod intake;
mod owner;
mod provenance;
mod setup;
mod state;
mod static_files;
mod telegram;
mod views;

use crate::event::Event;
use crate::pm::AppState;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};

/// Extract the current director's user_id from request extensions (injected by
/// `require_auth` middleware). Returns "ceo" as the default when unavailable —
/// fine for day 1 where there is exactly one director.
pub(crate) fn current_user_id(parts: &axum::http::request::Parts) -> String {
    parts
        .extensions
        .get::<crate::event::Actor>()
        .and_then(|a| match a {
            crate::event::Actor::Director { user_id } => Some(user_id.as_str()),
            _ => None,
        })
        .unwrap_or("ceo")
        .to_string()
}

/// Extractor for the current director's user_id. Handlers pull this instead of
/// hardcoding an identity, so the middleware-injected auth identity flows through.
pub(crate) struct CurrentUser(pub String);

impl<S> axum::extract::FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(CurrentUser(current_user_id(parts)))
    }
}

/// Log every incoming API request and its response status.
async fn log_request(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    log::info!("→ {} {}", method, uri);
    let response = next.run(req).await;
    log::info!("← {} {} → {}", method, uri, response.status().as_u16());
    response
}

use advisor::{advisor_handoff_handler, advisor_message_handler, advisor_summarize_handler};
use auth::{login_handler, require_auth};
use inbox::inbox_handler;
use intake::{brief_handler, diagram_handler, message_handler, request_handler};
use owner::{
    budget_handler, decision_handler, directive_handler, hire_handler, pause_handler,
    policy_handler, resume_handler,
};
use provenance::{provenance_commit_handler, provenance_decision_handler, provenance_task_handler};
use setup::{setup_handler, setup_status_handler};
use state::{events_handler, events_stream, health_handler, state_handler};
use static_files::static_handler;
use telegram::{telegram_configure_handler, telegram_status_handler};
use views::{
    consultants_handler, context_handler, full_context_handler, graph_handler,
    graph_task_context_handler, model_handler, persona_handler, routing_handler,
};

/// Shared helper: append a single event to the store and return it as the JSON
/// response. Collapses the repeated `state.append(ev)...Ok(Json(stored))`
/// boilerplate used by the director/advisor mutating handlers.
pub(crate) fn append_json(
    state: &AppState,
    ev: Event,
) -> Result<Json<Event>, (StatusCode, String)> {
    let stored = state
        .append(ev)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(stored))
}

/// Build the full router for a project's runtime state.
pub fn router(state: AppState) -> Router {
    // Owner-mutating endpoints are bearer-guarded when auth is enabled (the
    // middleware consults AppState.auth_token; no-op when it's None).
    let guarded = Router::new()
        .route("/api/message", axum::routing::post(message_handler))
        .route("/api/brief", axum::routing::post(brief_handler))
        .route("/api/request", axum::routing::post(request_handler))
        .route("/api/diagram", axum::routing::post(diagram_handler))
        .route(
            "/api/advisor/message",
            axum::routing::post(advisor_message_handler),
        )
        .route(
            "/api/advisor/handoff",
            axum::routing::post(advisor_handoff_handler),
        )
        .route(
            "/api/advisor/summarize",
            axum::routing::post(advisor_summarize_handler),
        )
        .route("/api/decision", axum::routing::post(decision_handler))
        .route("/api/policy", axum::routing::post(policy_handler))
        .route("/api/directive", axum::routing::post(directive_handler))
        .route("/api/hire", axum::routing::post(hire_handler))
        .route("/api/budget", axum::routing::post(budget_handler))
        .route("/api/pause", axum::routing::post(pause_handler))
        .route("/api/resume", axum::routing::post(resume_handler))
        // POST /api/setup and POST /api/telegram/configure are also owner-
        // mutating writes (they persist the director token / a bot secret), so
        // they get the same bearer guard. require_auth is a NO-OP when no
        // token is configured, so the first-run SetupWizard (which posts to
        // /api/setup before any token exists) keeps working; once a token is
        // set, these endpoints require it.
        .route("/api/setup", axum::routing::post(setup_handler))
        .route(
            "/api/telegram/configure",
            axum::routing::post(telegram_configure_handler),
        )
        // Auth-guarded GET endpoints: debug context exposes full system prompts,
        // and telegram status leaks the chat_id. Both need the director token.
        .route("/api/debug/context/{actor}", get(full_context_handler))
        .route("/api/telegram/status", get(telegram_status_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_auth,
        ));

    Router::new()
        .route("/api/login", axum::routing::post(login_handler))
        .route("/api/setup/status", get(setup_status_handler))
        .route("/api/state", get(state_handler))
        .route("/api/events", get(events_handler))
        .route("/api/events/stream", get(events_stream))
        .route("/api/inbox", get(inbox_handler))
        .merge(guarded)
        .route(
            "/api/provenance/commit/{sha}",
            get(provenance_commit_handler),
        )
        .route(
            "/api/provenance/task/{task_id}",
            get(provenance_task_handler),
        )
        .route(
            "/api/provenance/decision/{decision_id}",
            get(provenance_decision_handler),
        )
        .route("/api/context/{actor}", get(context_handler))
        // NOTE: /api/debug/context/{actor} is auth-guarded in the guarded router
        .route("/api/persona/{agent_id}", get(persona_handler))
        .route("/api/model", get(model_handler))
        .route("/api/graph", get(graph_handler))
        .route("/api/consultants", get(consultants_handler))
        .route("/api/routing", get(routing_handler))
        // NOTE: /api/telegram/status is auth-guarded in the guarded router
        .route("/api/health", get(health_handler))
        .route("/api/graph/task/{task_id}", get(graph_task_context_handler))
        // The embedded SPA (and SPA route fallback) handles everything else.
        .fallback(static_handler)
        .layer(axum::middleware::from_fn(log_request))
        .with_state(state)
}
