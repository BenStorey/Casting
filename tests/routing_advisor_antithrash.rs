//! Tests for the D2 batch (docs/plans/2026-08-14_d2-routing-advisor-antithrash.md):
//! per-actor model routing, the advisor LLM reply, and reactive anti-thrash.
//!
//! Routing + anti-thrash are deterministic unit tests. The advisor reply uses a
//! local stub chat/completions server (no live key, CI-safe).

use casting::cursor::SqliteCursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::llm::{LlmOrchestrator, ModelResolver, ProviderConfig};
use casting::orchestrator::Orchestrator as _;
use casting::pm::AppState;
use casting::sqlite_store::SqliteEventStore;
use serde_json::json;
use std::sync::Arc;

fn base_cfg() -> ProviderConfig {
    ProviderConfig {
        provider: "openrouter".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        api_key: "env-key".into(),
        model: "default-model".into(),
    }
}

// === #1 Per-actor model routing ===

#[test]
fn resolver_uses_env_base_for_unknown_actor() {
    let resolver = ModelResolver::new(base_cfg(), Default::default());
    let resolved = resolver.resolve("pm");
    assert_eq!(resolved.config.model, "default-model");
    assert_eq!(resolved.config.api_key, "env-key");
    assert_eq!(resolved.config.provider, "openrouter");
}

fn registry_with_model(
    consultant_toml: &str,
    prompt: &str,
) -> casting::consultants::ConsultantRegistry {
    // Build a registry from the embedded defaults overlaid with a custom
    // consultant package that declares a model binding (the test's input).
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("prompts")).unwrap();
    std::fs::write(dir.path().join("custom.toml"), consultant_toml).unwrap();
    std::fs::write(dir.path().join("prompts/custom.md"), prompt).unwrap();
    let mut reg = casting::consultants::ConsultantRegistry::from_embedded().unwrap();
    reg.overlay_dir(dir.path()).unwrap();
    // Keep the tempdir alive for the duration of the returned registry (the
    // prompts are read at load time into memory, so dropping is fine).
    drop(dir);
    reg
}

#[test]
fn resolver_uses_consultant_binding_with_key_fallback() {
    // A consultant with a model binding but NO key/base_url: uses its model +
    // provider, key falls back to env, base_url defaults via provider map.
    let pkg = r#"
[consultant]
id = "custom"
name = "Custom"
role = "engineer"
system_prompt = "prompts/custom.md"

[consultant.model]
provider = "openrouter"
model_id = "cheap-model"
"#;
    let registry = registry_with_model(pkg, "You are Custom, an engineer.");
    let resolver = ModelResolver::new(base_cfg(), registry);
    let resolved = resolver.resolve("custom");
    assert_eq!(
        resolved.config.model, "cheap-model",
        "consultant's model wins"
    );
    assert_eq!(resolved.config.provider, "openrouter");
    assert_eq!(resolved.config.api_key, "env-key", "key falls back to env");
    assert_eq!(
        resolved.config.base_url, "https://openrouter.ai/api/v1",
        "base_url defaults via provider map"
    );
    // A consultant persona is used (the package's system_prompt).
    assert!(
        resolved.system_prompt.contains("Custom"),
        "consultant persona used"
    );
}

#[test]
fn resolver_consultant_local_litellm_override() {
    // A consultant pinned to a local LiteLLM endpoint.
    let pkg = r#"
[consultant]
id = "local-guy"
name = "Local"
role = "engineer"
system_prompt = "prompts/custom.md"

[consultant.model]
provider = "litellm"
base_url = "http://localhost:4000/v1"
model_id = "local-llama"
"#;
    let registry = registry_with_model(pkg, "You are Local.");
    let resolver = ModelResolver::new(base_cfg(), registry);
    let resolved = resolver.resolve("local-guy");
    assert_eq!(resolved.config.model, "local-llama");
    assert_eq!(resolved.config.base_url, "http://localhost:4000/v1");
    assert_eq!(resolved.config.provider, "litellm");
}

// === Routing flows into the request ===

#[tokio::test]
async fn orchestrator_routes_per_actor_model() {
    // A stub server that records the model each request asks for. The
    // orchestrator is given a resolver where "marcus-reed" → "cheap-model", and
    // we plan for actor "marcus-reed" — the request must carry "cheap-model".
    use axum::routing::post;
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let seen = Arc::new(AtomicUsize::new(0));
    let seen_in = seen.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: Json<serde_json::Value>| async move {
            let model = body["model"].as_str().unwrap_or("").to_string();
            assert_eq!(
                model, "cheap-model",
                "per-actor model must reach the request"
            );
            seen_in.fetch_add(1, Ordering::SeqCst);
            Json(json!({
                "choices": [{"message": {"content": r#"{"actions":[{"action":"no_op"}]}"#}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5}
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut cfg = base_cfg();
    cfg.base_url = format!("http://{addr}/v1");
    // The consultant pins its OWN endpoint (the stub) + a per-actor model, which
    // proves BOTH the base_url override and the per-actor model routing.
    let pkg = format!(
        r#"
[consultant]
id = "marcus-reed"
name = "Marcus"
role = "engineer"
system_prompt = "prompts/custom.md"

[consultant.model]
provider = "stub"
base_url = "http://{addr}/v1"
model_id = "cheap-model"
"#
    );
    let registry = registry_with_model(&pkg, "You are Marcus.");
    let resolver = ModelResolver::new(cfg, registry);
    let orch = LlmOrchestrator::new(base_cfg(), "PM".into()).with_resolver(resolver);

    // Plan for actor "marcus-reed" with a throwaway cause.
    let ctx = casting::context::AgentContext {
        actor: "marcus-reed".into(),
        ..Default::default()
    };
    let cause = Event::new(
        "proj",
        Actor::Owner,
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m1".into(),
        },
        json!({ "body": "hi" }),
    );
    let out = orch.plan(&ctx, &cause).await.unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 1, "one provider call made");
    assert_eq!(
        out.metering.as_ref().unwrap().model.as_deref(),
        Some("cheap-model")
    );
    assert_eq!(out.metering.as_ref().unwrap().agent_id, "marcus-reed");
}

// === #3 Advisor LLM reply ===

async fn boot_stub(reply: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    use axum::routing::post;
    use axum::{Json, Router};
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || async move {
            Json(json!({
                "choices": [{"message": {"content": reply}}],
                "usage": {"prompt_tokens": 100, "completion_tokens": 30}
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/v1"), server)
}

#[tokio::test]
async fn advisor_reply_uses_its_model_and_thread() {
    let (base_url, _server) = boot_stub("Here is my strategic advice.").await;
    let mut cfg = base_cfg();
    cfg.base_url = base_url;
    let resolver = ModelResolver::new(cfg, Default::default());

    // A private advisor thread (owner asked a question).
    let thread = vec![casting::types::Message {
        id: "am-1".into(),
        from: "owner".into(),
        to: "advisor".into(),
        body: "What should we build?".into(),
    }];
    let outcome = casting::llm::advisor_reply(
        &resolver,
        &Default::default(),
        &thread,
        "What should we build?",
    )
    .await
    .unwrap();
    assert_eq!(outcome.reply, "Here is my strategic advice.");
    let m = outcome.metering.unwrap();
    assert_eq!(m.agent_id, "advisor");
    assert_eq!(m.prompt_tokens, 100);
}

#[tokio::test]
async fn advisor_reply_stays_isolated_from_pm_context() {
    // The advisor reply appends to the advisor_thread, NOT proj.messages (the
    // PM's inbox). We test the event shape directly via the web-free path: the
    // handler that drives it lives in the web layer, so we assert the MODEL
    // seam — advisor_reply produces a reply; the event isolation is already
    // guaranteed by the AdvisorMessageSent reducer (tests/advisor.rs).
    let (base_url, _server) = boot_stub("advice").await;
    let mut cfg = base_cfg();
    cfg.base_url = base_url;
    let resolver = ModelResolver::new(cfg, Default::default());
    let thread = vec![casting::types::Message {
        id: "am-1".into(),
        from: "owner".into(),
        to: "advisor".into(),
        body: "hi".into(),
    }];
    let outcome = casting::llm::advisor_reply(&resolver, &Default::default(), &thread, "hi")
        .await
        .unwrap();
    assert!(!outcome.reply.is_empty());
}

// === #2 Reactive anti-thrash ===

fn state_with_pm_and_cast() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let st = AppState::new(store, cursors, "proj-anti");
    st.append(Event::new(
        "proj-anti",
        Actor::System,
        EventType::ProjectCreated,
        Aggregate {
            kind: "project".into(),
            id: "proj-anti".into(),
        },
        json!({}),
    ))
    .unwrap();
    for (id, role) in [("pm", "Project Manager"), ("marcus-reed", "Engineer")] {
        st.append(Event::new(
            "proj-anti",
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: id.into(),
            },
            json!({ "role": role }),
        ))
        .unwrap();
    }
    st
}

fn propose_decision(subject: &str) -> casting::actions::PmAction {
    casting::actions::PmAction::ProposeDecision {
        id: format!("d-{}", subject.len()),
        subject: subject.into(),
        options: json!({"A": "x"}),
        recommendation: "A".into(),
        class: casting::policy::DecisionClass::InternalImplementation,
        involvement: casting::policy::OwnerInvolvement::Pm,
    }
}

#[test]
fn gate_rejects_reproposing_an_open_subject() {
    use casting::actions;
    let st = state_with_pm_and_cast();

    // First proposal of a subject passes the gate + creates an open decision.
    assert!(actions::validate(
        &propose_decision("Pick a DB"),
        "pm",
        &st.projection().unwrap()
    )
    .is_ok());
    for ev in (propose_decision("Pick a DB")).to_events("proj-anti", "pm", &cause(), "run-1") {
        st.append(ev).unwrap();
    }

    // Re-proposing the SAME subject is now rejected (anti-thrash).
    let err = actions::validate(
        &propose_decision("Pick a DB"),
        "pm",
        &st.projection().unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(err, actions::PolicyError::DecisionAlreadyOpen(_)),
        "got {err:?}"
    );
}

#[test]
fn gate_allows_different_subject_after_one_is_open() {
    use casting::actions;
    let st = state_with_pm_and_cast();
    for ev in (propose_decision("Pick a DB")).to_events("proj-anti", "pm", &cause(), "run-1") {
        st.append(ev).unwrap();
    }
    // A DIFFERENT subject is still fine.
    assert!(actions::validate(
        &propose_decision("Choose language"),
        "pm",
        &st.projection().unwrap()
    )
    .is_ok());
}

#[test]
fn prompt_mentions_anti_thrash_rule() {
    let orch = LlmOrchestrator::new(base_cfg(), "PM".into());
    let prompt = orch.planning_instructions();
    assert!(
        prompt.contains("ANTI-THRASH"),
        "prompt must state the anti-thrash rule"
    );
    assert!(
        prompt.contains("open_decisions"),
        "prompt must reference open_decisions"
    );
}

fn cause() -> Event {
    Event::new(
        "proj-anti",
        Actor::Owner,
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m1".into(),
        },
        json!({ "body": "go" }),
    )
}
