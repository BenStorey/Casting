//! Tests for the D2 batch (docs/plans/2026-08-14_d2-routing-advisor-antithrash.md):
//! per-actor model routing, the advisor LLM reply, and reactive anti-thrash.
//!
//! Routing + anti-thrash are deterministic unit tests. The advisor reply uses a
//! local stub chat/completions server (no live key, CI-safe).

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::llm::{LlmOrchestrator, ModelResolver, ProviderConfig};
use casting::pm::AppState;
use casting::runtime::orchestrator::Orchestrator as _;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
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
    let resolved = resolver.resolve("pm", None);
    assert_eq!(resolved.config.model, "default-model");
    assert_eq!(resolved.config.api_key, "env-key");
    assert_eq!(resolved.config.provider, "openrouter");
}

fn registry_with_model(
    consultant_toml: &str,
    prompt: &str,
) -> casting::consultants::ConsultantRegistry {
    // Build a registry from the embedded defaults overlaid with a custom
    // consultant package that carries inline system_prompt (no file).
    let tml = consultant_toml.replace("{INLINE_PROMPT}", prompt);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("custom.toml"), &tml).unwrap();
    let mut reg = casting::consultants::ConsultantRegistry::from_embedded().unwrap();
    reg.overlay_dir(dir.path()).unwrap();
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
cast_role = "lead_developer"
system_prompt = "{INLINE_PROMPT}"

[consultant.model]
provider = "openrouter"
model_id = "cheap-model"
"#;
    let registry = registry_with_model(pkg, "You are Custom, an engineer.");
    let resolver = ModelResolver::new(base_cfg(), registry);
    let resolved = resolver.resolve("custom", None);
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
cast_role = "lead_developer"
system_prompt = "{INLINE_PROMPT}"

[consultant.model]
provider = "litellm"
base_url = "http://localhost:4000/v1"
model_id = "local-llama"
"#;
    let registry = registry_with_model(pkg, "You are Local.");
    let resolver = ModelResolver::new(base_cfg(), registry);
    let resolved = resolver.resolve("local-guy", None);
    assert_eq!(resolved.config.model, "local-llama");
    assert_eq!(resolved.config.base_url, "http://localhost:4000/v1");
    assert_eq!(resolved.config.provider, "litellm");
}

#[test]
fn resolver_model_chain_uses_first_as_priority_and_orders_fallbacks() {
    // A multi-model chain: models[0] is preferred, the rest are fallbacks.
    let pkg = r#"
[consultant]
id = "chained"
name = "Chained"
cast_role = "lead_developer"
system_prompt = "{INLINE_PROMPT}"

[[consultant.models]]
provider = "openrouter"
model_id = "priority-model"
cost_tier = "premium"
temperature = 0.3

[[consultant.models]]
provider = "openrouter"
model_id = "fallback-1"
cost_tier = "standard"

[[consultant.models]]
provider = "openrouter"
model_id = "fallback-2"
cost_tier = "budget"
"#;
    let registry = registry_with_model(pkg, "You are Chained.");
    let resolver = ModelResolver::new(base_cfg(), registry);

    // `resolve` returns the FIRST (preferred) entry.
    let primary = resolver.resolve("chained", None);
    assert_eq!(primary.config.model, "priority-model");
    assert_eq!(primary.temperature, Some(0.3));

    // `resolve_chain` exposes the full ordered fallback chain.
    let chain = resolver.resolve_chain("chained");
    let ids: Vec<&str> = chain.iter().map(|r| r.config.model.as_str()).collect();
    assert_eq!(ids, vec!["priority-model", "fallback-1", "fallback-2"]);
    // Each link keeps its own tier prices (premium first → highest prices).
    assert!(chain[0].input_price_per_mtok > chain[1].input_price_per_mtok);
    assert!(chain[1].input_price_per_mtok > chain[2].input_price_per_mtok);
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
cast_role = "lead_developer"
system_prompt = "Marcus override prompt"

[consultant.model]
provider = "stub"
base_url = "http://{addr}/v1"
model_id = "cheap-model"
"#
    );
    let registry = registry_with_model(&pkg, "Marcus override prompt");
    let resolver = ModelResolver::new(cfg, registry);
    let orch = LlmOrchestrator::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("build reqwest client"),
        base_cfg(),
        "PM".into(),
    )
    .with_resolver(resolver);

    // Plan for actor "marcus-reed" with a throwaway cause.
    let ctx = casting::runtime::context::AgentContext {
        actor: "marcus-reed".into(),
        ..Default::default()
    };
    let cause = Event::new(
        "proj",
        Actor::Director {
            user_id: "ceo".into(),
        },
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
        from: "director".into(),
        to: "advisor".into(),
        body: "What should we build?".into(),
    }];
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("build reqwest client");
    let outcome = casting::llm::advisor_reply(
        &http_client,
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
        from: "director".into(),
        to: "advisor".into(),
        body: "hi".into(),
    }];
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("build reqwest client");
    let outcome =
        casting::llm::advisor_reply(&http_client, &resolver, &Default::default(), &thread, "hi")
            .await
            .unwrap();
    assert!(!outcome.reply.is_empty());
}

// === #1b Advisor grounded in the operating context ===

#[test]
fn advisor_context_summary_curates_high_level_state() {
    use casting::runtime::context::AgentContext;
    let ctx = AgentContext {
        actor: "advisor".into(),
        objective: Some("Ship the CLI".into()),
        active_directives: vec!["[policy] no telemetry".into()],
        open_risks: vec!["r1".into()],
        open_decisions: vec!["d1".into()],
        ..Default::default()
    };
    let summary = casting::llm::advisor::advisor_context_summary(&ctx);
    assert!(summary.contains("Ship the CLI"), "objective grounded");
    assert!(summary.contains("no telemetry"), "governance grounded");
    assert!(summary.contains("r1"), "risk grounded");
    assert!(
        !summary.contains("priorities"),
        "no task machinery for the advisor"
    );
}

#[tokio::test]
async fn advisor_reply_builds_grounding_into_system_prompt() {
    // The stub records the system prompt so we can assert the advisor's reply
    // call actually received the operating context.
    use axum::routing::post;
    use axum::{Json, Router};
    let recorded = Arc::new(MutexText::default());
    let rec = recorded.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: Json<serde_json::Value>| async move {
            let sys = body["messages"][0]["content"]
                .as_str()
                .unwrap_or("")
                .to_string();
            rec.0.lock().unwrap().push(sys);
            Json(json!({
                "choices": [{"message": {"content": "advice"}}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 3}
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut cfg = base_cfg();
    cfg.base_url = format!("http://{addr}/v1");
    let resolver = ModelResolver::new(cfg, Default::default());
    let ctx = casting::runtime::context::AgentContext {
        objective: Some("Ship the CLI".into()),
        ..Default::default()
    };
    let thread = vec![casting::types::Message {
        id: "am-1".into(),
        from: "director".into(),
        to: "advisor".into(),
        body: "advise me".into(),
    }];
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("build reqwest client");
    casting::llm::advisor_reply(&http_client, &resolver, &ctx, &thread, "advise me")
        .await
        .unwrap();
    let sys = recorded.0.lock().unwrap().join("\n");
    assert!(
        sys.contains("Ship the CLI"),
        "the advisor's reply prompt carried the objective"
    );
}

#[derive(Default)]
struct MutexText(std::sync::Mutex<Vec<String>>);

// === #2 Wiring temperature/max_tokens into the request ===

#[tokio::test]
async fn orchestrator_passes_consultant_temperature_and_max_tokens() {
    use axum::routing::post;
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let seen_temp = Arc::new(AtomicUsize::new(0));
    let st = seen_temp.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: Json<serde_json::Value>| async move {
            assert_eq!(body["temperature"], 0.7);
            assert_eq!(body["max_tokens"], 500);
            st.fetch_add(1, Ordering::SeqCst);
            Json(json!({
                "choices": [{"message": {"content": r#"{"actions":[{"action":"no_op"}]}"#}}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 3}
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut cfg = base_cfg();
    cfg.base_url = format!("http://{addr}/v1");
    let pkg = format!(
        r#"
[consultant]
id = "temp-guy"
name = "Temp"
cast_role = "lead_developer"
system_prompt = "{prompt}"

[consultant.model]
provider = "stub"
base_url = "http://{addr}/v1"
model_id = "temp-model"
temperature = 0.7
max_tokens = 500
"#,
        prompt = "Temp prompt"
    );
    let registry = registry_with_model(&pkg, "Temp prompt");
    let resolver = ModelResolver::new(cfg, registry);
    let orch = LlmOrchestrator::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("build reqwest client"),
        base_cfg(),
        "PM".into(),
    )
    .with_resolver(resolver);
    let ctx = casting::runtime::context::AgentContext {
        actor: "temp-guy".into(),
        ..Default::default()
    };
    let cause = Event::new(
        "proj",
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m1".into(),
        },
        json!({ "body": "hi" }),
    );
    orch.plan(&ctx, &cause).await.unwrap();
    assert_eq!(seen_temp.load(Ordering::SeqCst), 1);
}

// === #3 Routing surface (resolver-driven) ===

#[test]
fn resolver_round_trips_routing_surface_fields() {
    // The /api/routing view calls resolver.resolve(actor) for each actor; assert
    // the resolved struct exposes provider/model/base_url + temperature/max_tokens
    // (what the UI shows). The web handler itself is boot-tested in web_boot.rs.
    let pkg = r#"
[consultant]
id = "marcus-reed"
name = "Marcus"
cast_role = "lead_developer"
system_prompt = "{INLINE_PROMPT}"

[consultant.model]
provider = "openrouter"
model_id = "cheap-model"
temperature = 0.2
"#;
    let registry = registry_with_model(pkg, "You are Marcus.");
    let resolver = ModelResolver::new(base_cfg(), registry);
    let r = resolver.resolve("marcus-reed", None);
    assert_eq!(r.config.provider, "openrouter");
    assert_eq!(r.config.model, "cheap-model");
    assert_eq!(r.temperature, Some(0.2));
    // An actor with NO consultant binding → env base, no temp/max.
    let nobody = resolver.resolve("nobody-bound", None);
    assert_eq!(nobody.config.model, "default-model");
    assert_eq!(nobody.temperature, None);

    // The embedded PM package now binds pm to its own model (2026-08-14 cast).
    let pm = resolver.resolve("pm", None);
    assert_ne!(
        pm.config.model, "default-model",
        "pm ships a consultant binding"
    );
    assert_eq!(pm.config.model, "anthropic/claude-sonnet-5");
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
        class: casting::pm::policy::DecisionClass::InternalImplementation,
        involvement: casting::pm::policy::OwnerInvolvement::Pm,
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
        &st.projection().unwrap(),
        None,
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
        None,
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
        &st.projection().unwrap(),
        None,
    )
    .is_ok());
}

#[test]
fn prompt_mentions_anti_thrash_rule() {
    let orch = LlmOrchestrator::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("build reqwest client"),
        base_cfg(),
        "PM".into(),
    );
    let prompt = orch.planning_instructions("pm");
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
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m1".into(),
        },
        json!({ "body": "go" }),
    )
}

// === #4 Anti-thrash end-to-end through the LLM loop ===

#[tokio::test]
async fn llm_loop_rejects_and_audits_reproposing_an_open_subject_e2e() {
    // Drive the FULL PM loop with a stub model that, on the second pass,
    // re-proposes a decision subject that's already open. The gate must reject
    // it cleanly (a PlanActionRejected), no panic, and the audit trail shows it.
    use axum::routing::post;
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let calls = Arc::new(AtomicUsize::new(0));
    let c2 = calls.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |_body: Json<serde_json::Value>| async move {
            let n = c2.fetch_add(1, Ordering::SeqCst);
            // First pass: propose a decision. Second pass: re-propose the SAME
            // subject (anti-thrash should reject it).
            let content = if n == 0 {
                r#"{"actions":[
                    {"action":"propose_decision","id":"dec-1","subject":"Pick a DB","options":{"A":"pg","B":"sqlite"},"recommendation":"A","class":"internal_implementation","involvement":"pm"}
                ]}"#
            } else {
                r#"{"actions":[
                    {"action":"propose_decision","id":"dec-dup","subject":"Pick a DB","options":{"A":"pg"},"recommendation":"A","class":"internal_implementation","involvement":"pm"}
                ]}"#
            };
            Json(json!({
                "choices": [{"message": {"content": content}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5}
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut cfg = base_cfg();
    cfg.base_url = format!("http://{addr}/v1");
    let orch = LlmOrchestrator::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("build reqwest client"),
        cfg,
        "PM".into(),
    );
    let st = state_with_pm_and_cast()
        .with_orchestrator(Arc::new(orch))
        .with_step_delay(std::time::Duration::ZERO);

    // First owner message → the model proposes "Pick a DB" (accepted).
    st.append(Event::new(
        "proj-anti",
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m1".into(),
        },
        json!({ "body": "do it" }),
    ))
    .unwrap();
    casting::pm::drive_pm(&st).await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "first pass = one provider call"
    );
    let proj = st.projection().unwrap();
    assert!(
        proj.decisions.iter().any(|d| d.subject == "Pick a DB"),
        "first proposal landed"
    );

    // Second owner message → the model re-proposes the SAME subject. The gate
    // rejects it as PlanActionRejected; the loop does NOT panic.
    st.append(Event::new(
        "proj-anti",
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m2".into(),
        },
        json!({ "body": "again" }),
    ))
    .unwrap();
    casting::pm::drive_pm(&st).await.unwrap();

    // Only ONE open "Pick a DB" decision survives (the dup was rejected).
    let proj = st.projection().unwrap();
    let open_subject = proj
        .decisions
        .iter()
        .filter(|d| {
            d.subject == "Pick a DB" && d.status == casting::projection::DecisionStatus::Proposed
        })
        .count();
    assert_eq!(
        open_subject, 1,
        "no duplicate OPEN decision on the same subject"
    );

    // The rejected re-proposal is audited.
    let rejected = st
        .store
        .read_since("proj-anti", 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == EventType::PlanActionRejected)
        .count();
    assert!(
        rejected >= 1,
        "the duplicate re-proposal was audited as rejected"
    );
}

// === #1 Cost-tier prices: real spend + budget breaker ===

#[test]
fn tier_prices_map_is_ordered_and_nonzero() {
    use casting::consultants::CostTier;
    let (b_in, b_out) = casting::llm::routing::tier_prices(CostTier::Budget);
    let (s_in, s_out) = casting::llm::routing::tier_prices(CostTier::Standard);
    let (p_in, p_out) = casting::llm::routing::tier_prices(CostTier::Premium);
    assert!(b_in > 0.0 && b_out > 0.0, "budget prices non-zero");
    assert!(s_in > b_in, "standard input > budget input");
    assert!(p_in > s_in, "premium input > standard input");
    assert!(p_out > s_out, "premium output > standard output");
}

#[test]
fn resolver_carries_tier_prices() {
    // A Premium consultant resolves to premium prices; an unbound actor (pm)
    // defaults to Standard prices.
    let pkg = r#"
[consultant]
id = "prem-guy"
name = "Prem"
cast_role = "lead_developer"
system_prompt = "{INLINE_PROMPT}"

[consultant.model]
provider = "openrouter"
model_id = "prem-model"
cost_tier = "premium"
"#;
    let registry = registry_with_model(pkg, "You are Prem.");
    let resolver = ModelResolver::new(base_cfg(), registry);
    let prem = resolver.resolve("prem-guy", None);
    assert!(
        prem.input_price_per_mtok > 1.0,
        "premium input price elevated"
    );
    let pm = resolver.resolve("pm", None);
    assert!(
        (pm.input_price_per_mtok - 1.0).abs() < 1e-9,
        "unbound actor defaults to Standard input price"
    );
}

#[tokio::test]
async fn metering_reports_nonzero_usd_with_real_prices() {
    // Drive the LLM with a stub; assert the CostIncurred carries a NON-ZERO
    // estimated_usd (prompt 1200 * $1.00/M + completion 80 * $3.00/M), proving
    // the budget breaker now has real spend to trip on.
    use axum::routing::post;
    use axum::{Json, Router};
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || async move {
            Json(json!({
                "choices": [{"message": {"content": r#"{"actions":[{"action":"create_task","id":"tc","title":"X","kind":"feature"}]}"#}}],
                "usage": {"prompt_tokens": 1200, "completion_tokens": 80, "prompt_tokens_details": {"cached_tokens": 300}}
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut cfg = base_cfg();
    cfg.base_url = format!("http://{addr}/v1");
    let orch = LlmOrchestrator::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("build reqwest client"),
        cfg,
        "PM".into(),
    );
    let st = state_with_pm_and_cast()
        .with_orchestrator(Arc::new(orch))
        .with_step_delay(std::time::Duration::ZERO);
    st.append(Event::new(
        "proj-anti",
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m1".into(),
        },
        json!({ "body": "build it" }),
    ))
    .unwrap();
    casting::pm::drive_pm(&st).await.unwrap();

    let cost = st
        .store
        .read_since("proj-anti", 0)
        .unwrap()
        .into_iter()
        .find(|e| e.event_type == EventType::CostIncurred)
        .expect("a CostIncurred event");
    // Standard tier: in $1.00/M, out $3.00/M. prompt 1200, completion 80.
    let expected = (1200.0 * 1.00 + 80.0 * 3.00) / 1_000_000.0;
    let usd = cost.data["estimated_usd"].as_f64().unwrap();
    assert!(
        (usd - expected).abs() < 1e-9,
        "real metered spend: {usd} vs expected {expected}"
    );
    assert!(usd > 0.0, "spend is non-zero (budget breaker can trip)");
    assert_eq!(cost.data["input_price_per_mtok"], 1.0);
    assert_eq!(cost.data["output_price_per_mtok"], 3.0);
}

// === #2 Cache-write accounting ===

#[tokio::test]
async fn metering_threads_cache_write_tokens_from_provider() {
    // A provider that DOES report cache writes: the creation count must flow
    // into CostIncurred (not be silently dropped as hardcoded 0), and the
    // derived cache_hit_ratio must reflect reads/(reads+writes+fresh).
    use axum::routing::post;
    use axum::{Json, Router};
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || async move {
            Json(json!({
                "choices": [{"message": {"content": r#"{"actions":[{"action":"create_task","id":"cc","title":"X","kind":"feature"}]}"#}}],
                "usage": {
                    "prompt_tokens": 1000,
                    "completion_tokens": 10,
                    "prompt_tokens_details": {"cached_tokens": 100, "cache_creation": 200}
                }
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut cfg = base_cfg();
    cfg.base_url = format!("http://{addr}/v1");
    let orch = LlmOrchestrator::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("build reqwest client"),
        cfg,
        "PM".into(),
    );
    let st = state_with_pm_and_cast()
        .with_orchestrator(Arc::new(orch))
        .with_step_delay(std::time::Duration::ZERO);
    st.append(Event::new(
        "proj-anti",
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m1".into(),
        },
        json!({ "body": "go" }),
    ))
    .unwrap();
    casting::pm::drive_pm(&st).await.unwrap();

    let cost = st
        .store
        .read_since("proj-anti", 0)
        .unwrap()
        .into_iter()
        .find(|e| e.event_type == EventType::CostIncurred)
        .expect("a CostIncurred event");
    assert_eq!(
        cost.data["cache_read_input_tokens"], 100,
        "cache reads threaded"
    );
    assert_eq!(
        cost.data["cache_creation_input_tokens"], 200,
        "cache WRITES threaded (not hardcoded 0)"
    );

    // The derived operating picture's cache_hit_ratio accounts for writes:
    // reads / (fresh prompt + reads + writes) = 100 / (1000 + 100 + 200).
    let model = st.projection().unwrap().operating_model();
    let spend = &model.spend;
    let expected_ratio = 100.0 / (1000.0 + 100.0 + 200.0);
    assert!(
        (spend.cache_hit_ratio - expected_ratio).abs() < 1e-9,
        "ratio accounts for writes: {} vs {}",
        spend.cache_hit_ratio,
        expected_ratio
    );
    assert_eq!(spend.cache_creation_input_tokens, 200);
}

// === #1 Advisory briefings reach the operating context ===

#[test]
fn pm_context_carries_advisory_briefings() {
    // An advisor handoff / imported briefing must reach the PM's assembled
    // context (the thing the LLM is handed), clearly marked advisory — the
    // advisor→PM integration loop.
    let st = state_with_pm_and_cast();
    st.append(Event::new(
        "proj-anti",
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::AdvisoryBriefingImported,
        Aggregate {
            kind: "briefing".into(),
            id: "brief-1".into(),
        },
        json!({
            "source": "advisor",
            "subject": "direction",
            "title": "Open-core recommendation",
            "body": "Consider going open-core to grow adoption.",
        }),
    ))
    .unwrap();
    let ctx = st.projection().unwrap().context_for("pm");
    assert!(
        ctx.advisory_briefings
            .iter()
            .any(|b| b.contains("open-core")),
        "PM context includes the advisory briefing"
    );
    assert!(
        ctx.advisory_briefings
            .iter()
            .any(|b| b.contains("[advisory")),
        "briefing is explicitly marked advisory"
    );
    assert!(
        !ctx.advisory_briefings
            .iter()
            .any(|b| b.contains("[directive")),
        "advice is never confusable with a directive"
    );
}

#[test]
fn advisor_grounding_includes_prior_briefings() {
    use casting::runtime::context::AgentContext;
    let ctx = AgentContext {
        actor: "advisor".into(),
        advisory_briefings: vec!["[advisory · advisor] Open-core: go open-core".into()],
        ..Default::default()
    };
    let summary = casting::llm::advisor::advisor_context_summary(&ctx);
    assert!(
        summary.contains("open-core"),
        "advisor sees its own prior advice"
    );
    assert!(summary.contains("Prior advisory briefings"));
}

// === #2 LLM summary for the handoff ===

#[tokio::test]
async fn advisor_summarize_uses_the_model_but_never_hard_fails() {
    use axum::routing::post;
    use axum::{Json, Router};
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || async move {
            Json(json!({
                "choices": [{"message": {"content": "Concise distilled briefing."}}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 3}
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut cfg = base_cfg();
    cfg.base_url = format!("http://{addr}/v1");
    let resolver = ModelResolver::new(cfg, Default::default());
    let thread = vec![
        casting::types::Message {
            id: "m1".into(),
            from: "director".into(),
            to: "advisor".into(),
            body: "open-core or closed?".into(),
        },
        casting::types::Message {
            id: "m2".into(),
            from: "advisor".into(),
            to: "director".into(),
            body: "Consider open-core.".into(),
        },
    ];
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("build reqwest client");
    let summary = casting::llm::advisor_summarize(&http_client, &resolver, &thread)
        .await
        .unwrap();
    assert!(summary.contains("Concise"), "uses the model's summary");

    // Empty thread → deterministic fallback, no call needed.
    let http_client_empty = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .expect("build reqwest client");
    let empty = casting::llm::advisor_summarize(&http_client_empty, &resolver, &[])
        .await
        .unwrap();
    assert_eq!(empty, "Advisor conversation handed off to PM.");
}
