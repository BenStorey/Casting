//! Tests for owner auth (docs/PLAN: owner-auth).
//!
//! Bearer-token guarding of owner-mutating endpoints, opt-in. When no token is
//! configured auth is a no-op (backward compatible); when enabled, mutations
//! require `Authorization: Bearer <token>`.

use casting::cursor::CursorStore;
use casting::pm::AppState;
use casting::sqlite_store::SqliteEventStore;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

fn boot_api(auth: Option<&str>) -> axum::Router {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = CursorStore::in_memory().unwrap();
    let mut state = AppState::new(store, cursors, "proj-auth");
    if let Some(tok) = auth {
        state = state.with_owner_auth(tok);
    }
    casting::web::router(state)
}

async fn post(app: &axum::Router, path: &str, body: &str, token: Option<&str>) -> StatusCode {
    let mut req = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    if let Some(t) = token {
        req.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {t}").parse().unwrap(),
        );
    }
    app.clone().oneshot(req).await.unwrap().status()
}

// --- module-level ---

#[test]
fn bearer_token_parses_and_constant_time_equality() {
    use axum::http::HeaderMap;
    use casting::auth::{authorized, bearer_token};

    let mut m = HeaderMap::new();
    m.insert(header::AUTHORIZATION, "Bearer sekret".parse().unwrap());
    assert_eq!(bearer_token(&m), Some("sekret"));
    assert!(authorized(&m, "sekret"));
    assert!(!authorized(&m, "wrong"));
    // No header -> not authorized.
    assert!(!authorized(&HeaderMap::new(), "sekret"));
}

// --- web behavior ---

#[tokio::test]
async fn with_auth_mutation_requires_token() {
    let app = boot_api(Some("s3cr3t"));

    // Without a token: 401.
    let status = post(&app, "/api/message", r#"{"body":"hi"}"#, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // With the wrong token: 401.
    let status = post(&app, "/api/message", r#"{"body":"hi"}"#, Some("nope")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // With the right token: proceeds (200 OK here: message handler appends).
    let status = post(&app, "/api/message", r#"{"body":"hi"}"#, Some("s3cr3t")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn without_auth_mutation_is_not_guarded() {
    let app = boot_api(None);
    let status = post(&app, "/api/message", r#"{"body":"hi"}"#, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "no auth configured -> mutations open"
    );
}

#[tokio::test]
async fn login_accepts_correct_and_rejects_wrong_token() {
    let app = boot_api(Some("s3cr3t"));
    let ok = post(&app, "/api/login", r#"{"token":"s3cr3t"}"#, None).await;
    assert_eq!(ok, StatusCode::OK);
    let bad = post(&app, "/api/login", r#"{"token":"wrong"}"#, None).await;
    assert_eq!(bad, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn read_endpoints_stay_open_with_auth_enabled() {
    let app = boot_api(Some("s3cr3t"));
    let req = Request::builder()
        .uri("/api/state")
        .body(Body::empty())
        .unwrap();
    let status = app.clone().oneshot(req).await.unwrap().status();
    assert_eq!(
        status,
        StatusCode::OK,
        "reads stay open even when auth is on"
    );
}
