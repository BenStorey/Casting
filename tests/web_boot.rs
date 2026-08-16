//! Regression tests that BOOT the HTTP router.
//!
//! Motivation: the committed Git slice shipped provenance routes written with
//! axum 0.7 capture syntax (`/api/provenance/commit/:sha`) while the project
//! runs axum 0.8, which requires `{param}`. axum rejects `:param` at router
//! *build* time, so `cast run` panicked immediately on startup — yet the whole
//! suite (45 tests) still passed, because the provenance tests exercise the
//! pure query functions and never construct the web router. This file closes
//! that coverage gap.
//!
//! Building the router is itself the assertion that the route table is valid;
//! the `oneshot` requests verify the endpoints actually answer.

use casting::pm::policy::{DecisionClass, OwnerInvolvement};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::FutureExt;
use tower::ServiceExt;

fn boot_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-boot")
}

/// The module under test is `casting::web`, which is why the router type is
/// `casting::web::router` (not something private to this crate).
#[test]
fn router_builds_without_panicking() {
    // If any route uses invalid axum 0.8 syntax, this call panics (it did:
    // the old `:capture` form). Constructing it here is the regression guard.
    let state = boot_state();
    let _app = casting::web::router(state);
}

#[test]
fn provenance_routes_are_mounted_and_answer() {
    // axum 0.8 capture-group syntax must be `{param}`, not `:param`. Boot the
    // full router so a future regression throws immediately, then confirm the
    // two provenance endpoints are reachable (200 for a known-but-missing id is
    // fine — the point is the route is mounted and dispatch does not 404/panic).
    let state = boot_state();
    let app = casting::web::router(state);

    for path in [
        "/api/provenance/commit/deadbeef",
        "/api/provenance/task/task-501",
        "/api/provenance/decision/decision-db",
        "/api/state",
        "/api/inbox",
        "/api/policy",
        "/api/directive",
        "/api/hire",
        "/api/setup/status",
        "/api/context/owner",
        "/api/model",
        "/api/graph",
        "/api/consultants",
        "/api/routing",
        "/api/brief",
        "/api/request",
        "/api/diagram",
        "/api/advisor/message",
        "/api/advisor/handoff",
        "/api/advisor/summarize",
        "/api/budget",
        "/api/pause",
        "/api/resume",
        "/api/telegram/status",
        "/api/telegram/configure",
        "/api/health",
    ] {
        let req = Request::builder().uri(path).body(Body::empty()).unwrap();
        let resp = app
            .clone()
            .oneshot(req)
            .now_or_never()
            .expect("router dispatch should not block")
            .expect("router oneshot is infallible");
        let status = resp.status();
        assert!(
            !status.is_server_error(),
            "route {path} should not 5xx (got {status})"
        );
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "route {path} should be mounted (got 404)"
        );
    }
}

#[test]
fn policy_endpoint_records_and_folds_an_owner_policy_change() {
    let state = boot_state();
    let app = casting::web::router(state.clone());

    // Owner escalates SecurityCritical to Ask via the endpoint.
    let body = serde_json::json!({
        "class": "security_critical",
        "involvement": "ask",
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/policy")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app
        .clone()
        .oneshot(req)
        .now_or_never()
        .expect("dispatch should not block")
        .expect("infallible");
    assert_eq!(resp.status(), StatusCode::OK);

    // The event is durable and the projection's policy reflects it.
    let proj = Projection::build(&state.store, "proj-boot").unwrap();
    assert_eq!(
        proj.policy.resolve(DecisionClass::SecurityCritical),
        OwnerInvolvement::Ask
    );

    // And the gate now rejects an under-claiming proposal for that class.
    use casting::actions::validate;
    use casting::actions::PolicyError;
    let err = validate(
        &casting::actions::PmAction::ProposeDecision {
            id: "d".into(),
            subject: "security".into(),
            options: serde_json::json!({}),
            recommendation: "x".into(),
            class: DecisionClass::SecurityCritical,
            involvement: OwnerInvolvement::Notify,
        },
        "pm",
        &proj,
        None,
    )
    .expect_err("under-claiming a class the owner escalated to Ask must be rejected");
    assert!(matches!(
        err,
        PolicyError::AuthorityDowngrade {
            class: DecisionClass::SecurityCritical,
            required: OwnerInvolvement::Ask,
            claimed: OwnerInvolvement::Notify,
        }
    ));
}
