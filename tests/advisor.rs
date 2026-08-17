//! Tests for the Direction Advisor (owner 2026-08-10): a special second role the
//! owner chats with directly. The advisor thread is ISOLATED from PM context by
//! design until an explicit handoff, which becomes an AdvisoryBriefing the PM
//! reads (provenanced "advisor").

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

fn state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-advisor")
}

fn advisor_message(st: &AppState, body: &str) {
    st.append(Event::new(
        &st.project,
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::AdvisorMessageSent,
        Aggregate {
            kind: "advisor_thread".into(),
            id: format!("am-{}", body.len()),
        },
        serde_json::json!({ "to": "advisor", "body": body }),
    ))
    .unwrap();
}

#[test]
fn advisor_thread_is_recorded_separately_from_pm_messages() {
    let st = state();
    // An owner message to the PM and a director message to the advisor.
    st.append(Event::new(
        &st.project,
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m1".into(),
        },
        serde_json::json!({ "to": "pm", "body": "build me a todo app" }),
    ))
    .unwrap();
    advisor_message(&st, "how do we think about pricing?");
    advisor_message(&st, "maybe open-core with a hosted tier");

    let proj = Projection::build(&st.store, &st.project).unwrap();
    assert_eq!(
        proj.messages.len(),
        1,
        "PM thread has exactly the 1 PM message"
    );
    assert_eq!(proj.messages[0].body, "build me a todo app");
    assert_eq!(
        proj.advisor_thread.len(),
        2,
        "advisor thread has the 2 advisory messages"
    );
    assert_eq!(proj.advisor_thread[0].from, "owner");
    assert_eq!(proj.advisor_thread[0].to, "advisor");
}

#[test]
fn advisor_thread_does_not_enter_operating_context_until_handoff() {
    let st = state();
    advisor_message(&st, "should we go open-core or closed?");

    // Before any handoff: the operating model has NO advisor briefing.
    let proj = Projection::build(&st.store, &st.project).unwrap();
    let m = proj.operating_model();
    assert_eq!(
        m.knowledge.briefings.active_count, 0,
        "advisor chat must NOT reach PM context"
    );

    // Hand off: it becomes an advisor briefing the PM reads.
    st.append(Event::new(
        &st.project,
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::AdvisorHandoff,
        Aggregate {
            kind: "briefing".into(),
            id: "brief-1".into(),
        },
        serde_json::json!({
            "source": "advisor",
            "subject": "pricing strategy",
            "title": "Open-core vs closed",
            "body": "Recommend open-core with a hosted tier.",
        }),
    ))
    .unwrap();

    let proj2 = Projection::build(&st.store, &st.project).unwrap();
    assert_eq!(proj2.briefings.len(), 1);
    let b = &proj2.briefings[0];
    assert_eq!(b.source, "advisor", "provenanced as advisor");
    assert_eq!(b.title, "Open-core vs closed");
    assert_eq!(b.body, "Recommend open-core with a hosted tier.");

    let m2 = proj2.operating_model();
    assert_eq!(
        m2.knowledge.briefings.active_count, 1,
        "handoff briefing now in PM context"
    );
    assert!(m2.knowledge.briefings.active[0].contains("advisor"));
}

#[tokio::test]
async fn web_advisor_endpoints_work() {
    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use tower::ServiceExt;

    let st = state();
    let app: Router = casting::web::router(st.clone());

    // Owner -> advisor message.
    let msg = serde_json::json!({ "body": "should we raise prices?" });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/advisor/message")
                .header("content-type", "application/json")
                .body(Body::from(msg.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Handoff -> advisor briefing.
    let ho = serde_json::json!({
        "summary": "Raise prices for the hosted tier.",
        "title": "Pricing"
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/advisor/handoff")
                .header("content-type", "application/json")
                .body(Body::from(ho.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "handoff should succeed");

    let proj = Projection::build(&st.store, &st.project).unwrap();
    assert_eq!(proj.advisor_thread.len(), 1);
    assert_eq!(proj.briefings.len(), 1);
    assert_eq!(proj.briefings[0].source, "advisor");
}
