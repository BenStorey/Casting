//! Tests for external advisor briefings (director 2026-08-10) — importing content
//! from OUTSIDE Casting (e.g. a ChatGPT plan) as ADVISORY context that can
//! inform but never sets rules. Covers the event/projection path, the gate
//! action, and the /api/brief endpoint.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::{BriefingStatus, Projection};
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

fn state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-brief")
}

#[test]
fn briefing_reduces_into_projection_with_provenance() {
    let st = state();
    st.append(Event::new(
        &st.project,
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::AdvisoryBriefingImported,
        Aggregate {
            kind: "briefing".into(),
            id: "brief-1".into(),
        },
        serde_json::json!({
            "source": "ChatGPT advisor",
            "subject": "architecture",
            "title": "chatgpt D2 plan",
            "body": "Proposed split of the increment into D2 phases...",
            "assets": [ { "caption": "architecture", "location": "diagrams/d2.png" } ],
            "brought_in_by": "ceo",
            "supersedes": null,
        }),
    ))
    .unwrap();

    let proj = Projection::build(&st.store, &st.project).unwrap();
    assert_eq!(proj.briefings.len(), 1);
    let b = &proj.briefings[0];
    assert_eq!(b.id, "brief-1");
    assert_eq!(b.source, "ChatGPT advisor"); // provenance preserved
    assert_eq!(b.subject, "architecture");
    assert_eq!(b.assets.len(), 1);
    assert_eq!(b.assets[0].location, "diagrams/d2.png");
    assert_eq!(b.status, BriefingStatus::Active);
}

#[test]
fn import_briefing_action_through_gate_and_to_events() {
    let st = state();
    let action = casting::actions::PmAction::ImportBriefing {
        id: "brief-9".into(),
        source: "jeeves".into(),
        subject: "storage".into(),
        title: "storage notes".into(),
        body: "Postgres is a reasonable default.".into(),
        assets: vec![],
    };
    // Not authoritative — passes the gate (no cross-entity invariant).
    casting::actions::validate(&action, "director", &Projection::default(), None).unwrap();

    let cause = Event::new(
        &st.project,
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m".into(),
        },
        serde_json::json!({}),
    );
    let evs = action.to_events(&st.project, "owner", &cause, "brief", None);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].event_type, EventType::AdvisoryBriefingImported);
    assert_eq!(evs[0].aggregate.id, "brief-9");
    assert_eq!(evs[0].data["source"], "jeeves");
    // Advisory flag is implicit: brought_in_by records who, source records where.
    assert_eq!(evs[0].data["brought_in_by"], "owner");
}

#[test]
fn operating_model_surfaces_advisory_briefings_separately() {
    let st = state();
    st.append(Event::new(
        &st.project,
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::AdvisoryBriefingImported,
        Aggregate {
            kind: "briefing".into(),
            id: "brief-1".into(),
        },
        serde_json::json!({
            "source": "ChatGPT advisor",
            "subject": "architecture",
            "title": "D2 plan",
            "body": "Split the increment into D2 phases",
            "assets": [],
            "brought_in_by": "ceo",
            "supersedes": null,
        }),
    ))
    .unwrap();

    let proj = Projection::build(&st.store, &st.project).unwrap();
    let m = proj.operating_model();
    assert_eq!(m.knowledge.briefings.active_count, 1);
    assert_eq!(m.knowledge.briefings.active.len(), 1);
    assert!(m.knowledge.briefings.active[0].contains("ChatGPT advisor"));
    assert!(m.knowledge.briefings.active[0].contains("D2 plan"));
    assert!(m.knowledge.briefings.superseded.is_empty());
}

#[tokio::test]
async fn web_brief_endpoint_imports_advisory_content() {
    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use tower::ServiceExt;

    // Build the web router (the same one that runs a project).
    let st = state();
    let app: Router = casting::web::router(st.clone());

    let body = serde_json::json!({
        "source": "ChatGPT advisor",
        "subject": "databases",
        "title": "foundationdb notes",
        "body": "FoundationDB's ordered key-value maps well to our event log.",
        "assets": [ { "caption": "layout", "location": "https://example.com/x.png" } ]
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/brief")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "brief import should succeed");

    // The briefing landed in the projection, advisory + authoritative separate.
    let proj = Projection::build(&st.store, &st.project).unwrap();
    assert_eq!(proj.briefings.len(), 1);
    let b = &proj.briefings[0];
    assert_eq!(b.source, "ChatGPT advisor");
    assert_eq!(b.subject, "databases");
    assert_eq!(b.assets[0].location, "https://example.com/x.png");
    // Nothing authoritative was created: no directive, no assumption.
    assert!(proj.directives.is_empty());
    assert!(proj.assumptions.is_empty());
}
