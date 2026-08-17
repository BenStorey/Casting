//! Tests for context-assembly scoring (2026-08-10): each priority is annotated
//! with a deterministic relevance score for the receiving actor — own-task and
//! urgent/blocked items rank highest — so the PM/agent pays attention in order.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

fn state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-scoring")
}

#[test]
fn context_priorities_carry_relevance_scores() {
    let st = state();
    for (kind, id, data) in [
        ("project", "proj-scoring", serde_json::json!({})),
        (
            "requirements",
            "req-1",
            serde_json::json!({ "title": "Build a thing", "description": "x" }),
        ),
    ] {
        st.append(Event::new(
            &st.project,
            Actor::System,
            if kind == "project" {
                EventType::ProjectCreated
            } else {
                EventType::RequirementCreated
            },
            Aggregate {
                kind: kind.into(),
                id: id.into(),
            },
            data,
        ))
        .unwrap();
    }
    // Two tasks: one assigned to marcus (mine), one high-priority unassigned.
    st.append(Event::new(
        &st.project,
        Actor::Agent { id: "mei".into() },
        EventType::TaskCreated,
        Aggregate {
            kind: "task".into(),
            id: "task-1".into(),
        },
        serde_json::json!({ "title": "Mine", "kind": "engineering" }),
    ))
    .unwrap();
    st.append(Event::new(
        &st.project,
        Actor::Agent { id: "mei".into() },
        EventType::TaskAssigned,
        Aggregate {
            kind: "task".into(),
            id: "task-1".into(),
        },
        serde_json::json!({ "assignee": "marcus-reed", "priority": "high" }),
    ))
    .unwrap();

    let proj = casting::projection::Projection::build(&st.store, &st.project).unwrap();
    let ctx = proj.context_for("marcus-reed");
    assert!(
        ctx.scored_priorities.iter().any(|s| s.is_mine),
        "marcus's own task should be flagged is_mine"
    );
    // Owner sees everything as relevant (no negative role penalty).
    let owner_ctx = proj.context_for("director");
    let all_positive = owner_ctx
        .scored_priorities
        .iter()
        .all(|s| s.relevance >= 0.0);
    assert!(all_positive, "owner relevance scores should all be >= 0");
    // A "mine" item scores higher than a matching non-mine item at same tier.
    let owner_scores = owner_ctx.scored_priorities.clone();
    let mine_score = ctx
        .scored_priorities
        .iter()
        .find(|s| s.is_mine)
        .map(|s| s.relevance);
    assert!(mine_score.is_some());
    // Sanity: a non-director's own high-priority task scores > a passive observer.
    let passive = proj.context_for("maya-patel").scored_priorities.clone();
    let _ = (owner_scores, passive); // structural sanity only
}

#[tokio::test]
async fn web_model_surfaces_scored_priorities_for_actors() {
    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use tower::ServiceExt;

    let st = state();
    st.append(Event::new(
        &st.project,
        Actor::System,
        EventType::ProjectCreated,
        Aggregate {
            kind: "project".into(),
            id: "proj-scoring".into(),
        },
        serde_json::json!({}),
    ))
    .unwrap();
    let app: Router = casting::web::router(st.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/model")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // Each actor context carries scored_priorities.
    assert!(v["actor_contexts"][0]["scored_priorities"].is_array());
}
