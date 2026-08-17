//! Owner authentication (docs/PLAN: owner-auth).
//!
//! Scoped to auth ALONE: a single owner bearer token guarding the
//! owner-mutating API endpoints. Opt-in via `AppState::with_owner_auth` /
//! the `CAST_DIRECTOR_TOKEN` env var. No multi-project workspaces here.
//!
//! The token is a long, high-entropy secret (not a user-chosen password), so a
//! constant-time compare is the right level of protection — no password hashing
//! needed (that would be over-engineering for a bearer secret).

use axum::http::HeaderMap;

/// The `Authorization` header value we expect.
const BEARER_PREFIX: &str = "Bearer ";

/// Constant-time equality for a bearer secret.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Extract the bearer token from the request's Authorization header, if present
/// and well-formed.
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    auth.strip_prefix(BEARER_PREFIX)
}

/// True if the request carries the expected owner token.
pub fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    match bearer_token(headers) {
        Some(tok) => constant_time_eq(tok, expected),
        None => false,
    }
}
