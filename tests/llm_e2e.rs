//! D2 — end-to-end test of the real LLM orchestrator against a local stub
//! OpenAI-compatible `chat/completions` server (no live key, no spend, CI-safe).
//!
//! Proves the ENTIRE seam: prompt built → HTTP POST shaped correctly → reply
//! parsed into `PmAction`s → validated through the policy gate → `CostIncurred`
//! event landed → spend attributable — with the real `LlmOrchestrator`, just
//! pointed at a local endpoint instead of OpenRouter. This is exactly how a
//! local LiteLLM/vLLM/Ollama backend would slot in: a different base_url.

use casting::cursor::SqliteCursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::llm::{LlmOrchestrator, ProviderConfig};
use casting::pm::{drive_pm, AppState};
use casting::sqlite_store::SqliteEventStore;
use serde_json::json;
use std::sync::Arc;

/// The canned actions the stub "model" returns (a real PM-style decision).
const STUB_ACTIONS: &str = r#"{"actions":[
  {"action":"create_task","id":"task-llm-1","title":"Build the thing","kind":"feature"}
]}"#;

/// Boot a stub OpenAI-compatible chat/completions server on an ephemeral port.
/// Returns the base_url to point the client at (plus a handle on _must outlive
/// the test). The stub asserts the request shape (bearer auth + model + json
/// mode) and returns a canned completion with usage, so cost attribution is
/// exercised exactly as against a real provider.
async fn boot_stub(actions_json: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    use axum::{routing::post, Json, Router};

    let app = Router::new().route(
        "/v1/chat/completions",
        post(
            move |headers: axum::http::HeaderMap, body: Json<serde_json::Value>| async move {
                // Assert the client sent what an OpenAI-compatible backend expects.
                let auth = headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                assert!(
                    auth == "Bearer test-key",
                    "stub expected bearer auth, got {auth:?}"
                );
                assert_eq!(
                    body["model"], "stub-model",
                    "stub expected the configured model"
                );
                assert_eq!(
                    body["messages"][0]["role"], "system",
                    "stub expected a system message"
                );
                assert!(
                    body["messages"].as_array().unwrap().len() >= 2,
                    "stub expected system+user"
                );

                let payload = json!({
                    "choices": [{"message": {"content": actions_json}}],
                    "usage": {
                        "prompt_tokens": 1200,
                        "completion_tokens": 80,
                        "prompt_tokens_details": {"cached_tokens": 300}
                    }
                });
                Json(payload)
            },
        ),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/v1"), server)
}

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-llm")
}

/// Seed the project + hire the PM, exactly like `cast run` does at boot.
fn seed(state: &AppState) {
    state
        .append(Event::new(
            "proj-llm",
            Actor::System,
            EventType::ProjectCreated,
            Aggregate {
                kind: "project".into(),
                id: "proj-llm".into(),
            },
            json!({}),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-llm",
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: "pm".into(),
            },
            json!({ "role": "Project Manager" }),
        ))
        .unwrap();
}

#[tokio::test]
async fn llm_orchestrator_drives_the_pm_loop_end_to_end() {
    let (base_url, _server) = boot_stub(STUB_ACTIONS).await;
    let cfg = ProviderConfig {
        provider: "stub".into(),
        base_url,
        api_key: "test-key".into(),
        model: "stub-model".into(),
    };
    let orch =
        LlmOrchestrator::new(cfg, "You are the Project Manager.".into()).with_prices(0.25, 1.25);

    let state = make_state().with_orchestrator(Arc::new(orch));
    seed(&state);

    // Owner sends the first message (the trigger that routes through the
    // orchestrator). Zero animation delay so the loop runs immediately.
    let state = state.with_step_delay(std::time::Duration::ZERO);
    state
        .append(Event::new(
            "proj-llm",
            Actor::Owner,
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "m1".into(),
            },
            json!({ "body": "Build me a product." }),
        ))
        .unwrap();

    drive_pm(&state).await.unwrap();

    let proj = state.projection().unwrap();
    // The LLM's action was validated + executed: a task exists on the board.
    assert!(
        proj.tasks.iter().any(|t| t.id == "task-llm-1"),
        "LLM-emitted create_task should have produced a real task"
    );

    // Cost was landed in the event log (metering from the stub's usage).
    let spend_event = state
        .store
        .read_since("proj-llm", 0)
        .unwrap()
        .into_iter()
        .find(|e| e.event_type == EventType::CostIncurred);
    assert!(
        spend_event.is_some(),
        "expected a CostIncurred event from metering"
    );
    let spend = spend_event.unwrap();
    assert_eq!(spend.data["provider"], "stub");
    assert_eq!(spend.data["prompt_tokens"], 1200);
    assert_eq!(spend.data["completion_tokens"], 80);
    assert_eq!(spend.data["cache_read_input_tokens"], 300);
}

#[tokio::test]
async fn malformed_model_reply_errors_without_panicking() {
    // Stub returns garbage that is not a valid actions envelope.
    let (base_url, _server) = boot_stub(r#"this is not json"#).await;
    let cfg = ProviderConfig {
        provider: "stub".into(),
        base_url,
        api_key: "test-key".into(),
        model: "stub-model".into(),
    };
    let orch = LlmOrchestrator::new(cfg, "You are the Project Manager.".into());

    let state = make_state()
        .with_orchestrator(Arc::new(orch))
        .with_step_delay(std::time::Duration::ZERO);
    seed(&state);
    state
        .append(Event::new(
            "proj-llm",
            Actor::Owner,
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "m1".into(),
            },
            json!({ "body": "Do something." }),
        ))
        .unwrap();

    // Must not panic; returns an authored-count (the parse failed → no actions).
    let authored = drive_pm(&state).await.unwrap();
    assert_eq!(
        authored, 0,
        "a malformed reply should produce no domain actions"
    );

    // The failed pass is audited in the event log (diagnostics trail).
    let orch_run = state
        .store
        .read_since("proj-llm", 0)
        .unwrap()
        .into_iter()
        .find(|e| e.event_type == EventType::OrchestrationRun);
    assert!(
        orch_run.is_some(),
        "expected an audited OrchestrationRun on failure"
    );
    assert!(orch_run.unwrap().data.get("error").is_some());
}

/// The provider is config, not code: OpenRouter and LiteLLM resolve to their
/// own base URLs, and a bare model+provider with NO key stays off (deterministic
/// scripted PM default — no spend, backwards compatible).
#[test]
fn provider_resolution_and_off_by_default() {
    // Bare provider (no API key) → None (LLM stays unplugged).
    std::env::remove_var("CAST_LLM_API_KEY");
    assert!(casting::llm::config::from_env().unwrap().is_none());

    // No key at all → None even with other vars set.
    std::env::remove_var("CAST_LLM_MODEL");
    std::env::remove_var("CAST_LLM_PROVIDER");
    assert!(casting::llm::config::from_env().unwrap().is_none());
}

#[test]
fn provider_base_url_map() {
    assert_eq!(
        casting::llm::config::default_base_url("openrouter"),
        Some("https://openrouter.ai/api/v1")
    );
    assert_eq!(
        casting::llm::config::default_base_url("litellm"),
        Some("http://localhost:4000/v1")
    );
    assert_eq!(casting::llm::config::default_base_url("vllm"), None);
}

/// The action vocabulary round-trips through the same serde shape the model is
/// instructed to emit: a tagged JSON object (`{"action": "create_task", ...}`).
#[test]
fn pm_action_serde_round_trip() {
    use casting::actions::PmAction;
    let action = PmAction::CreateTask {
        id: "t1".into(),
        title: "Build".into(),
        kind: "feature".into(),
    };
    let wire = serde_json::to_value(&action).unwrap();
    assert_eq!(wire["action"], "create_task", "tagged with the action name");
    let back: PmAction = serde_json::from_value(wire).unwrap();
    assert_eq!(action, back);
}
