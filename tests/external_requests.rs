//! Tests for the ExternalRequest intake surface (director 2026-08-10): the
//! product's pickup of issues/PRs raised outside — recorded with provenance,
//! triaged deterministically (classification/severity/dedup), NEVER as the
//! director's own intent.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::{ExternalRequestStatus, Projection};
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

fn state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-req")
}

fn append_request(st: &AppState, id: &str, source: &str, title: &str, labels: Vec<&str>) {
    st.append(Event::new(
        &st.project,
        // The request comes FROM OUTSIDE — the pm records it, not the director.
        Actor::Agent { id: "pm".into() },
        EventType::ExternalRequestReceived,
        Aggregate {
            kind: "external_request".into(),
            id: id.into(),
        },
        serde_json::json!({
            "source": source,
            "external_id": format!("{id}-ext"),
            "title": title,
            "body": "",
            "reporter": "alice",
            "labels": labels,
            "url": format!("https://github.com/x/{id}"),
            "classification": "bug",
            "severity": "medium",
        }),
    ))
    .unwrap();
}

#[test]
fn external_request_reduces_with_provenance_and_triage() {
    let st = state();
    append_request(&st, "req-1", "github", "App crashes on login", vec!["bug"]);

    let proj = Projection::build(&st.store, &st.project).unwrap();
    assert_eq!(proj.external_requests.len(), 1);
    let r = &proj.external_requests[0];
    assert_eq!(r.source, "github");
    assert_eq!(r.reporter, "alice");
    assert_eq!(r.classification, "bug");
    assert_eq!(r.severity, "medium");
    assert_eq!(r.status, ExternalRequestStatus::Open);
    // Provenance preserved, distinct from director intent.
    assert!(r.external_id.is_some());
}

#[test]
fn triage_classifies_security_bug_feature_and_severity() {
    let st = state();
    // Empty projection: classification/severity from the heuristic alone.
    let proj = Projection::build(&st.store, &st.project).unwrap();

    let (c1, s1, _) = proj.triage_request(
        "github",
        None,
        "Remote code execution in auth",
        "",
        &["security".into()],
    );
    assert_eq!(c1, "security");
    assert_eq!(s1, "high");

    let (c2, s2, _) = proj.triage_request("github", None, "Crash when opening settings", "", &[]);
    assert_eq!(c2, "bug");
    assert_eq!(s2, "high", "crash words bump severity to high");

    let (c3, s3, _) =
        proj.triage_request("github", None, "Add dark mode", "", &["enhancement".into()]);
    assert_eq!(c3, "feature");
    assert_eq!(s3, "low");
}

#[test]
fn triage_detects_duplicates_by_external_id_and_title() {
    let st = state();
    append_request(&st, "req-1", "github", "App crashes on login", vec!["bug"]);

    let proj = Projection::build(&st.store, &st.project).unwrap();
    // Same external id -> duplicate.
    let (_, _, dup1) =
        proj.triage_request("github", Some("req-1-ext"), "App crashes on login", "", &[]);
    // Same normalized title -> duplicate.
    let (_, _, dup2) = proj.triage_request("github", None, "app crashes on login", "", &[]);
    // Different title -> not a dup.
    let (_, _, dup3) = proj.triage_request("github", None, "Add export feature", "", &[]);

    assert!(dup1, "same external_id should dedup");
    assert!(dup2, "same title (case-insensitive) should dedup");
    assert!(!dup3, "different title should not dedup");
}

#[test]
fn operating_model_surfaces_open_requests() {
    let st = state();
    append_request(&st, "req-1", "github", "Crash on login", vec!["bug"]);
    let proj = Projection::build(&st.store, &st.project).unwrap();

    let m = proj.operating_model();
    assert_eq!(m.requests.open_count, 1);
    assert_eq!(m.requests.open.len(), 1);
    assert!(m.requests.open[0].contains("bug"));
    assert!(m.requests.open[0].contains("Crash on login"));
    assert!(m.requests.open[0].contains("alice"));
}

#[tokio::test]
async fn web_request_endpoint_records_and_triages() {
    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use tower::ServiceExt;

    let st = state();
    let app: Router = casting::web::router(st.clone());
    let body = serde_json::json!({
        "source": "github",
        "external_id": "42",
        "title": "App crashes on startup",
        "body": "Can't reproduce but happens for some users",
        "reporter": "bob",
        "labels": ["bug", "repro-needed"],
        "url": "https://github.com/x/issues/42"
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/request")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "request intake should succeed");

    let proj = Projection::build(&st.store, &st.project).unwrap();
    assert_eq!(proj.external_requests.len(), 1);
    let r = &proj.external_requests[0];
    assert_eq!(r.source, "github");
    assert_eq!(r.reporter, "bob");
    assert_eq!(r.labels, vec!["bug", "repro-needed"]);
    assert_eq!(r.classification, "bug", "triage should run on intake");
}
