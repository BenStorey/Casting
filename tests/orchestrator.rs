//! Tests for the orchestrator seam (D2) and its MockOrchestrator implementation.
//!
//! The mock is a stateless test double: it produces deterministic actions but
//! does NOT record metering/cost data (real cost tracking happens through the
//! LlmOrchestrator in production). Tests that need cost tracking should use
//! a real provider client or a different test seam.

use casting::actions::PmAction;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::runtime::context::AgentContext;
use casting::runtime::orchestrator::MockOrchestrator;
use casting::store::EventStore;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
use std::sync::Arc;
use std::time::Duration;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-orch")
        .with_step_delay(Duration::ZERO)
        .with_orchestrator(Arc::new(MockOrchestrator))
}

/// Seed a requirement so the mock has an objective (else it only acks).
fn seed_requirement(state: &AppState) {
    state
        .append(Event::new(
            "proj-orch",
            Actor::System,
            EventType::RequirementCreated,
            Aggregate {
                kind: "requirement".into(),
                id: "req-1".into(),
            },
            serde_json::json!({ "title": "Build a thing", "body": "Build a thing" }),
        ))
        .unwrap();
    state
        .cursors
        .advance(
            "proj-orch",
            "mei",
            state.store.latest_sequence("proj-orch").unwrap(),
        )
        .unwrap();
}

#[tokio::test]
async fn orchestrator_plans_actions_from_context() {
    let state = make_state();
    seed_requirement(&state);

    state
        .append(Event::new(
            "proj-orch",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-1".into(),
            },
            serde_json::json!({ "body": "Build me a thing" }),
        ))
        .unwrap();

    let authored = casting::pm::drive_pm(&state).await.unwrap();
    assert!(
        authored > 0,
        "mock should author at least one action per plan"
    );
}

#[tokio::test]
async fn mock_does_not_record_cost() {
    // The MockOrchestrator is a stateless test double — it does NOT
    // record metering/cost data. This test verifies that invariant.
    let state = make_state();
    seed_requirement(&state);
    state
        .append(Event::new(
            "proj-orch",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-1".into(),
            },
            serde_json::json!({ "body": "Build me a thing" }),
        ))
        .unwrap();

    casting::pm::drive_pm(&state).await.unwrap();
    let proj = Projection::build(&state.store, "proj-orch").unwrap();
    assert!(
        proj.spend.is_empty(),
        "a stateless mock must not record cost (got {} entries)",
        proj.spend.len()
    );
}

#[tokio::test]
async fn orchestrator_records_a_planning_run_in_diagnostics() {
    // The planning diagnostic (OrchestrationRun event) is always recorded,
    // regardless of metering. Only the metering-specific fields change.
    // Use a DecisionMade trigger (not MessageSent) because owner messages
    // now take the deterministic chat-interface path and skip the orchestrator.
    let state = make_state();
    seed_requirement(&state);
    // Seed a budget so the budget gate doesn't block the orchestrator call.
    state
        .append(Event::new(
            "proj-orch",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::BudgetSet,
            Aggregate {
                kind: "budget".into(),
                id: "budget".into(),
            },
            serde_json::json!({ "limit_usd": 100.0, "warn_at": 0.80 }),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-orch",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::DecisionMade,
            Aggregate {
                kind: "decision".into(),
                id: "msg-1".into(),
            },
            serde_json::json!({ "subject": "test", "approved": true, "note": "Build me a thing", "body": "Build me a thing" }),
        ))
        .unwrap();

    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-orch").unwrap();
    assert_eq!(
        proj.orchestration.len(),
        1,
        "one orchestrator planning pass should be recorded"
    );
    let run = &proj.orchestration[0];
    assert_eq!(run.trigger, "DecisionMade");
    assert_eq!(run.actor, "mei");
    assert!(
        run.context_summary.contains("objective="),
        "context summary tells the reader what was handed in: {}",
        run.context_summary
    );
    assert!(
        run.planned.iter().any(|p| p.contains("\"create_task\"")),
        "recorded what the model decided to do: {:?}",
        run.planned
    );
    // The mock is stateless — no metering is reported.
    assert!(!run.metered, "a stateless mock reports no metering");
    assert_eq!(run.provider, None);
    assert_eq!(run.estimated_usd, 0.0);
}

/// A test orchestrator that reports the assembled prompt + raw response (like
/// the real `LlmOrchestrator`), so the archival wiring can be exercised end to
/// end without a live provider.
#[derive(Debug, Clone, Copy, Default)]
struct PromptOrch;

#[async_trait::async_trait]
impl casting::runtime::orchestrator::Orchestrator for PromptOrch {
    async fn plan(
        &self,
        context: &AgentContext,
        _cause: &Event,
    ) -> anyhow::Result<casting::runtime::orchestrator::PlanOutput> {
        let pm = context.pm_id.clone();
        Ok(casting::runtime::orchestrator::PlanOutput {
            actions: vec![(
                pm.clone(),
                PmAction::SendMessage {
                    to: "director".into(),
                    body: "ok".into(),
                },
            )],
            metering: None,
            prompt: Some("SYSTEM x\nUSER y".into()),
            response: Some("{\"actions\":[]}".into()),
        })
    }
}

#[tokio::test]
async fn orchestrator_persists_prompt_and_response_to_archive() {
    // Attach a prompt archive rooted at a temp dir and drive a planning pass
    // with an orchestrator that reports its prompt/response.
    let tmp = std::env::temp_dir().join(format!("casting-orch-archive-{}", uuid::Uuid::new_v4()));
    let state = AppState::new(
        SqliteEventStore::in_memory().unwrap(),
        SqliteCursorStore::in_memory().unwrap(),
        "proj-orch",
    )
    .with_step_delay(Duration::ZERO)
    .with_orchestrator(Arc::new(PromptOrch))
    .with_prompt_archive(Some(
        casting::workspace::prompt_archive::PromptArchive::open(&tmp),
    ));
    seed_requirement(&state);
    state
        .append(Event::new(
            "proj-orch",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::BudgetSet,
            Aggregate {
                kind: "budget".into(),
                id: "budget".into(),
            },
            serde_json::json!({ "limit_usd": 100.0, "warn_at": 0.80 }),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-orch",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::DecisionMade,
            Aggregate {
                kind: "decision".into(),
                id: "msg-1".into(),
            },
            serde_json::json!({ "subject": "test", "approved": true, "body": "Build a thing" }),
        ))
        .unwrap();

    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-orch").unwrap();
    assert_eq!(proj.orchestration.len(), 1, "one planning pass recorded");
    let run = &proj.orchestration[0];

    // The refs are recorded on the OrchestrationRun projection.
    let prompt_ref = run.prompt_ref.as_deref().expect("prompt ref recorded");
    let response_ref = run.response_ref.as_deref().expect("response ref recorded");

    // And the blobs actually landed under the archive dir with the right content.
    let archive = casting::workspace::prompt_archive::PromptArchive::open(&tmp);
    assert_eq!(
        std::fs::read_to_string(archive.resolve(prompt_ref).unwrap()).unwrap(),
        "SYSTEM x\nUSER y"
    );
    assert_eq!(
        std::fs::read_to_string(archive.resolve(response_ref).unwrap()).unwrap(),
        "{\"actions\":[]}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
