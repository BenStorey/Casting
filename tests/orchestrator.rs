//! Tests for the D2 orchestrator seam (docs/PLAN: A — mocked provider).
//!
//! The real LLM is deliberately UNPLUGGED (off by default, no spend). The
//! MockOrchestrator proves the seam end-to-end: context -> PmActions -> gate ->
//! events, with zero live model.

use casting::cursor::SqliteCursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::orchestrator::MockOrchestrator;
use casting::pm::AppState;
use casting::projection::Projection;
use casting::sqlite_store::SqliteEventStore;
use std::sync::Arc;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-orch")
}

#[test]
fn orchestrator_is_off_by_default() {
    let state = make_state();
    assert!(
        state.orchestrator.is_none(),
        "real LLM must be unplugged by default"
    );
}

#[tokio::test]
async fn mock_orchestrator_drives_the_pm_loop_end_to_end() {
    let state = make_state().with_orchestrator(Arc::new(MockOrchestrator));
    // Seed the project + hire the PM (what main.rs does on startup).
    state
        .append(Event::new(
            "proj-orch",
            Actor::System,
            EventType::ProjectCreated,
            Aggregate {
                kind: "project".into(),
                id: "proj-orch".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-orch",
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: "pm".into(),
            },
            serde_json::json!({ "role": "Project Manager" }),
        ))
        .unwrap();

    // Owner sends the first message.
    state
        .append(Event::new(
            "proj-orch",
            Actor::Owner,
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-1".into(),
            },
            serde_json::json!({ "body": "Build me a thing" }),
        ))
        .unwrap();

    // Drive the PM: the mock orchestrator responds (not the scripted plan).
    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-orch").unwrap();
    // The mock acknowledged the owner with a MessageSent from PM.
    assert!(
        proj.messages
            .iter()
            .any(|m| m.from == "pm" && m.to == "owner" && m.body.contains("Build me a thing")),
        "mock orchestrator should acknowledge the owner message"
    );

    // Now establish an objective (a requirement), then the next owner message
    // drives the mock to actually BUILD a task from context.
    state
        .append(Event::new(
            "proj-orch",
            Actor::Agent { id: "pm".into() },
            EventType::RequirementCreated,
            Aggregate {
                kind: "requirement".into(),
                id: "req-1".into(),
            },
            serde_json::json!({ "title": "Build me a thing", "description": "x" }),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-orch",
            Actor::Owner,
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-2".into(),
            },
            serde_json::json!({ "body": "go" }),
        ))
        .unwrap();
    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-orch").unwrap();
    assert!(
        proj.tasks.iter().any(|t| t.id == "task-mock-1"),
        "with an objective, the mock orchestrator plans a task from context"
    );
}

#[tokio::test]
async fn orchestrator_metering_lands_cost_in_the_event_log() {
    // HARNESS #6: when the orchestrator reports provider metering, the PM lands
    // a CostIncurred event so spend is attributable per agent/task from the
    // source of truth (not tracked separately). The mock emits metering when it
    // actually "plans," so drive it past the ack to a planning call.
    let state = make_state().with_orchestrator(Arc::new(MockOrchestrator));
    for (etype, id, kind, data) in [
        (
            EventType::ProjectCreated,
            "proj-orch",
            "project",
            serde_json::json!({}),
        ),
        (
            EventType::AgentHired,
            "pm",
            "agent",
            serde_json::json!({ "role": "Project Manager" }),
        ),
        // A requirement establishes an objective so the next owner message
        // actually triggers the mock's planning branch (which meters).
        (
            EventType::RequirementCreated,
            "req-1",
            "requirement",
            serde_json::json!({ "title": "R", "description": "x" }),
        ),
        (
            EventType::MessageSent,
            "msg-1",
            "message",
            serde_json::json!({ "body": "Build me a thing" }),
        ),
    ] {
        state
            .append(Event::new(
                "proj-orch",
                if etype == EventType::MessageSent {
                    Actor::Owner
                } else {
                    Actor::System
                },
                etype,
                Aggregate {
                    kind: kind.into(),
                    id: id.into(),
                },
                data,
            ))
            .unwrap();
    }

    // Drive the PM: with an objective present, the mock plans AND reports
    // metering (~1200 prompt + 300 completion tokens, $0.0018).
    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-orch").unwrap();
    assert_eq!(
        proj.spend.len(),
        1,
        "mock planning call should land exactly one CostIncurred entry"
    );
    let entry = &proj.spend[0];
    assert_eq!(entry.agent_id, "pm");
    assert_eq!(entry.model_tier, "flash");
    assert_eq!(entry.model, Some("deepseek/deepseek-v4-flash-0731".into()));
    assert_eq!(entry.provider, Some("openrouter".into()));
    assert_eq!(entry.prompt_tokens, 1200);
    assert_eq!(entry.completion_tokens, 300);
    assert_eq!(
        entry.cache_read_input_tokens, 200,
        "per-call cache reads round-trip"
    );
    assert_eq!(
        entry.cache_creation_input_tokens, 100,
        "per-call cache creation round-trips"
    );
    assert_eq!(
        entry.latency_ms, 150,
        "per-call latency round-trips into the entry"
    );
    assert_eq!(entry.input_price_per_mtok, Some(0.25));
    assert_eq!(entry.output_price_per_mtok, Some(1.25));
    assert!(entry.estimated_usd > 0.0);

    // The budget view in the operating picture reflects it.
    let m = proj.operating_model();
    assert_eq!(m.spend.entries, 1);
    assert!((m.spend.total_estimated_usd - 0.0018).abs() < 1e-9);
    assert_eq!(m.spend.by_agent.get("pm"), Some(&0.0018));
    assert_eq!(m.spend.cache_read_input_tokens, 200);
    assert_eq!(m.spend.cache_creation_input_tokens, 100);
    assert!(
        (m.spend.cache_hit_ratio - (200.0 / 1500.0)).abs() < 1e-9,
        "hit ratio is reads / (prompt + reads + creation)"
    );
    assert_eq!(m.spend.avg_latency_ms, Some(150.0));
}
