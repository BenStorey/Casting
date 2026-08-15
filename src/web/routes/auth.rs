use crate::pm::AppState;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

/// Auth middleware for owner-mutating endpoints: when `AppState.auth_token` is
/// set, require `Authorization: Bearer *** otherwise pass through (auth
/// disabled, backward compatible with tests / local runs).
pub(crate) async fn require_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    if let Some(expected) = state.auth_token.clone() {
        if !crate::workspace::auth::authorized(req.headers(), &expected) {
            return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
        }
    }
    next.run(req).await
}

#[derive(Deserialize)]
pub(crate) struct LoginIn {
    token: String,
}

/// POST /api/login {token} — verify an owner token (200 ok) or not (401). Lets a
/// UI validate the token the user pasted before using it for mutations.
pub(crate) async fn login_handler(
    State(state): State<AppState>,
    Json(input): Json<LoginIn>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.auth_token.as_deref() {
        Some(expected)
            if crate::workspace::auth::authorized(&fake_headers_with(&input.token), expected) =>
        {
            Ok(Json(serde_json::json!({ "ok": true })))
        }
        // If auth is disabled entirely, any token is accepted (nothing to guard).
        None => Ok(Json(serde_json::json!({ "ok": true }))),
        Some(_) => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Build a single-header map carrying a bearer token (used by login, which gets
/// the token in the body rather than the header).
fn fake_headers_with(token: &str) -> axum::http::HeaderMap {
    use axum::http::header::AUTHORIZATION;
    let mut m = axum::http::HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&format!("Bearer {token}")) {
        m.insert(AUTHORIZATION, v);
    }
    m
}
