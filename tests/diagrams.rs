//! Tests for the diagram artifact (owner 2026-08-10): drawing inside the app is
//! saved DIRECTLY from the tldraw editor as a durable, reloadable visual
//! artifact — no export/re-upload. DiagramSaved event + projection + endpoint.

use casting::cursor::SqliteCursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::sqlite_store::SqliteEventStore;

fn state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-diagram")
}

fn append_diagram(st: &AppState, id: &str, title: &str, data: &str) {
    st.append(Event::new(
        &st.project,
        Actor::Owner,
        EventType::DiagramSaved,
        Aggregate {
            kind: "diagram".into(),
            id: id.into(),
        },
        serde_json::json!({
            "title": title,
            "data": data,
            "saved_by": "owner",
        }),
    ))
    .unwrap();
}

#[test]
fn diagram_reduces_with_title_and_data() {
    let st = state();
    append_diagram(&st, "diagram-1", "Auth flow", "{\"store\":{}}");

    let proj = Projection::build(&st.store, &st.project).unwrap();
    assert_eq!(proj.diagrams.len(), 1);
    let d = &proj.diagrams[0];
    assert_eq!(d.id, "diagram-1");
    assert_eq!(d.title, "Auth flow");
    assert_eq!(d.data, "{\"store\":{}}");
    assert_eq!(d.saved_by, "owner");
}

#[test]
fn operating_model_lists_diagrams() {
    let st = state();
    append_diagram(&st, "diagram-1", "Auth flow", "x");

    let proj = Projection::build(&st.store, &st.project).unwrap();
    let m = proj.operating_model();
    assert_eq!(m.diagrams.count, 1);
    assert_eq!(m.diagrams.diagrams.len(), 1);
    assert!(m.diagrams.diagrams[0].contains("Auth flow"));
    assert!(m.diagrams.diagrams[0].contains("owner"));
}

#[tokio::test]
async fn web_diagram_endpoint_saves_directly() {
    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use tower::ServiceExt;

    let st = state();
    let app: Router = casting::web::router(st.clone());
    let body = serde_json::json!({
        "title": "UI sketch",
        "data": "{\"store\":{\"x\":1}}"
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/diagram")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "diagram save should succeed");

    let proj = Projection::build(&st.store, &st.project).unwrap();
    assert_eq!(proj.diagrams.len(), 1);
    let d = &proj.diagrams[0];
    assert_eq!(d.title, "UI sketch");
    assert_eq!(d.data, "{\"store\":{\"x\":1}}");
}
