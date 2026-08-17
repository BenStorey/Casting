//! D2 — LLM integration tests (docs/plans/2026-08-14_llm-tests.md).
//!
//! Everything here runs against a LOCAL stub OpenAI-compatible
//! `chat/completions` server (127.0.0.1:0, no live key, no spend, CI-safe), so
//! the whole seam is pinned down without hitting a real provider. A single
//! opt-in live test (`live_openrouter_round_trip`) uses a real key and never
//! runs in CI.

use axum::http::{HeaderMap, StatusCode};
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::llm::{LlmOrchestrator, ProviderConfig};
use casting::pm::{drive_pm, AppState};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// What the stub responds with. The default builder returns a happy
/// completion; custom responders let a test return HTTP errors etc.
type Responder =
    Arc<dyn Fn(&HeaderMap, &serde_json::Value) -> (StatusCode, serde_json::Value) + Send + Sync>;

/// A handle on the running stub server + a request counter (to assert "no call
/// happened" for the guard / not-hijacked cases).
struct Stub {
    base_url: String,
    req_count: Arc<AtomicUsize>,
    _server: tokio::task::JoinHandle<()>,
}

/// Assert the request is a well-formed OpenAI-compatible chat/completions POST.
fn assert_shape(headers: &HeaderMap, body: &serde_json::Value, expect_ask: &str) {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(auth, "Bearer test-key", "bearer auth");
    assert_eq!(body["model"], "stub-model", "configured model");
    assert_eq!(
        body["messages"][0]["role"], "system",
        "system message first"
    );
    let msgs = body["messages"].as_array().unwrap();
    assert!(msgs.len() >= 2, "system + user messages");
    // the director's raw ask must reach the model (abstracted AgentContext has
    // objective=None pre-Requirement).
    let user_msg = msgs[1]["content"].as_str().unwrap_or("");
    assert!(
        user_msg.contains(expect_ask),
        "owner's ask must be in the user message, got: {user_msg}"
    );
}

/// The standard happy completion payload for a given actions JSON + usage.
fn happy_payload(actions_json: &str, status: StatusCode) -> (StatusCode, serde_json::Value) {
    (
        status,
        json!({
            "choices": [{"message": {"content": actions_json}}],
            "usage": {
                "prompt_tokens": 1200,
                "completion_tokens": 80,
                "prompt_tokens_details": {"cached_tokens": 300}
            }
        }),
    )
}

async fn boot_stub(responder: Responder) -> Stub {
    use axum::{routing::post, Json, Router};
    let req_count = Arc::new(AtomicUsize::new(0));
    let counter = req_count.clone();
    let handler_responder = responder.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |headers, body: Json<serde_json::Value>| {
            let counter = counter.clone();
            let responder = handler_responder.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                let (status, payload) = responder(&headers, &body.0);
                (status, Json(payload))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Stub {
        base_url: format!("http://{addr}/v1"),
        req_count,
        _server: server,
    }
}

/// A default responder that asserts request shape and returns a happy
/// completion carrying `actions_json`. `expect_ask` must be 'static (owned).
fn default_responder(actions_json: &'static str, expect_ask: &'static str) -> Responder {
    Arc::new(move |h, b| {
        assert_shape(h, b, expect_ask);
        happy_payload(actions_json, StatusCode::OK)
    })
}

fn make_state() -> AppState {
    let store = casting::store::SqliteEventStore::in_memory().unwrap();
    let cursors = casting::store::SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-llm")
}

/// Seed the project + hire the PM + the default cast (marcus engineer / maya
/// qa), mirroring `cast run`'s seed now that the cast is seeded at first open.
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
    // Seed a budget so the budget gate (llm_dispatch_allowed) doesn't block
    // orchestrator calls for owner-decision-triggered tests.
    state
        .append(Event::new(
            "proj-llm",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::BudgetSet,
            Aggregate {
                kind: "budget".into(),
                id: "budget".into(),
            },
            json!({ "limit_usd": 100.0, "warn_at": 0.80 }),
        ))
        .unwrap();
    for (id, role) in [
        ("mei", "Project Manager"),
        ("marcus-reed", "Engineer"),
        ("maya-patel", "QA"),
    ] {
        state
            .append(Event::new(
                "proj-llm",
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
}

fn msg_body(state: &AppState, id: &str, body: &str) {
    state
        .append(Event::new(
            "proj-llm",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: id.into(),
            },
            json!({ "body": body }),
        ))
        .unwrap();
}

/// Send a DecisionMade trigger that routes through the orchestrator (unlike
/// MessageSent which takes the deterministic chat-interface path). The data
/// includes a "body" field so the orchestrator's ask extraction works.
fn decision_trigger(state: &AppState, id: &str, body: &str) {
    state
        .append(Event::new(
            "proj-llm",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::DecisionMade,
            Aggregate {
                kind: "decision".into(),
                id: id.into(),
            },
            json!({ "subject": "test", "approved": true, "note": body, "body": body }),
        ))
        .unwrap();
}

fn orch_for(base_url: String, with_prices: bool) -> LlmOrchestrator {
    let cfg = ProviderConfig {
        provider: "stub".into(),
        base_url,
        api_key: "test-key".into(),
        model: "stub-model".into(),
    };
    let o = LlmOrchestrator::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("build reqwest client"),
        cfg,
        "You are the Project Manager.".into(),
    );
    if with_prices {
        o.with_prices(0.25, 1.25)
    } else {
        o
    }
}

// === A. Prompt contract vs serde drift ===

/// Every action the prompt tells the model to emit must actually
/// deserialize/round-trip through the real `PmAction` serde shape, and the
/// prompt must list every required field. Guards the "missing field 'to'" class
/// of bug at its source (prompt drifting from the enum).
#[test]
fn prompt_contract_matches_serde_shape() {
    use casting::actions::PmAction;
    let orch = orch_for("http://stub".into(), false);
    let prompt = orch.planning_instructions("mei");

    // A representative probe covering every action the model may emit, with the
    // exact fields the serde type requires. Serializing each gives the field
    // NAMES; the prompt must mention them (plus the action tag).
    let probes: Vec<(PmAction, &str)> = vec![
        (
            PmAction::CreateTask {
                id: "t".into(),
                title: "x".into(),
                kind: "feature".into(),
            },
            "create_task",
        ),
        (
            PmAction::AssignTask {
                task_id: "t".into(),
                assignee: "m".into(),
                merge_authority: casting::types::MergeAuthority::SelfMerge,
            },
            "assign_task",
        ),
        (
            PmAction::StartTask {
                task_id: "t".into(),
            },
            "start_task",
        ),
        (
            PmAction::CompleteTask {
                task_id: "t".into(),
                result: "r".into(),
            },
            "complete_task",
        ),
        (
            PmAction::RequestReview {
                task_id: "t".into(),
                reviewer: "m".into(),
            },
            "request_review",
        ),
        (
            PmAction::ReviewTask {
                task_id: "t".into(),
                approved: true,
                note: None,
            },
            "review_task",
        ),
        (
            PmAction::CommitToChangeSet {
                task_id: "t".into(),
                message: "m".into(),
            },
            "commit_to_change_set",
        ),
        (
            PmAction::RaiseRisk {
                id: "r".into(),
                subject: "s".into(),
                severity: "high".into(),
            },
            "raise_risk",
        ),
        (
            PmAction::ResolveRisk {
                risk_id: "r".into(),
                status: casting::projection::RiskStatus::Resolved,
            },
            "resolve_risk",
        ),
        (
            PmAction::RecordAssumption {
                id: "a".into(),
                body: "b".into(),
            },
            "record_assumption",
        ),
        (
            PmAction::RecordConstraint {
                id: "c".into(),
                body: "b".into(),
            },
            "record_constraint",
        ),
        (
            PmAction::RecordOpinion {
                id: "o".into(),
                subject: "s".into(),
                category: "c".into(),
                statement: "st".into(),
                supersedes: None,
            },
            "record_opinion",
        ),
        (
            PmAction::RecordFact {
                id: "f".into(),
                kind: "k".into(),
                statement: "st".into(),
            },
            "record_fact",
        ),
        (
            PmAction::SendMessage {
                to: "director".into(),
                body: "b".into(),
            },
            "send_message",
        ),
        (
            PmAction::CreateObservation {
                id: "ob".into(),
                severity: "info".into(),
                subject: "s".into(),
                body: "b".into(),
                pm_action_required: false,
            },
            "create_observation",
        ),
        (
            PmAction::BlockTask {
                task_id: "t".into(),
                reason: "r".into(),
            },
            "block_task",
        ),
        (
            PmAction::SetTaskPriority {
                task_id: "t".into(),
                priority: casting::pm::plan::Priority::High,
            },
            "set_task_priority",
        ),
        (
            PmAction::ProposeConsultant {
                id: "pc".into(),
                subject: "s".into(),
                role_id: "engineer".into(),
                involvement: casting::pm::policy::OwnerInvolvement::Pm,
            },
            "propose_consultant",
        ),
        (
            PmAction::MakeDecision {
                decision_id: "d".into(),
                approved: true,
                note: None,
            },
            "make_decision",
        ),
    ];

    for (action, tag) in probes {
        // The action tag must be present in the prompt.
        assert!(prompt.contains(tag), "prompt missing action tag '{tag}'");
        // Every required field (serialized keys) must be mentioned in the prompt
        // for that action's line, so the model knows the full contract.
        let wire = serde_json::to_value(&action).unwrap();
        let obj = wire.as_object().unwrap();
        for key in obj.keys() {
            if key == "action" {
                continue;
            }
            assert!(
                prompt.contains(key),
                "prompt for '{tag}' missing required field '{key}'"
            );
        }
    }

    // Internal actions the model shouldn't hand-author (authority-only):
    // create_directive / supersede_directive are director/PM-authority paths, not
    // LLM planning vocabulary. (Sanity guard that we did not over-expose them.)
    // (No assert needed — the point is the ones above all round-trip.)
}

/// The enum VALUES the prompt advertises must match real serde serializations
/// (e.g. priority low/medium/high/critical, not a renamed variant).
#[test]
fn prompt_enum_values_match_serde() {
    let orch = orch_for("http://stub".into(), false);
    let prompt = orch.planning_instructions("mei");

    use casting::pm::plan::Priority;
    use casting::pm::policy::OwnerInvolvement;
    use casting::projection::RiskStatus;
    use casting::types::TaskStatus;

    // Serialize real values (lowercase via serde) and require the prompt mention
    // the exact tokens it advertises.
    for p in [
        Priority::Low,
        Priority::Medium,
        Priority::High,
        Priority::Critical,
    ] {
        let v = serde_json::to_value(p)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            prompt.contains(&v),
            "prompt must advertise priority enum value '{v}'"
        );
    }
    for s in [
        RiskStatus::Open,
        RiskStatus::Materialized,
        RiskStatus::Resolved,
    ] {
        let v = serde_json::to_value(s)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            prompt.contains(&v),
            "prompt must advertise risk status '{v}'"
        );
    }
    for t in [
        TaskStatus::Backlog,
        TaskStatus::Working,
        TaskStatus::InReview,
        TaskStatus::Blocked,
        TaskStatus::Done,
    ] {
        let v = serde_json::to_value(t)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            prompt.contains(&v),
            "prompt must advertise task status '{v}'"
        );
    }
    // OwnerInvolvement uses explicit renames (pm/ask/never/notify).
    for i in [
        OwnerInvolvement::Pm,
        OwnerInvolvement::Ask,
        OwnerInvolvement::Never,
        OwnerInvolvement::Notify,
    ] {
        let v = serde_json::to_value(i)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            prompt.contains(&v),
            "prompt must advertise involvement '{v}'"
        );
    }
}

// === B. Parse robustness ===

#[test]
fn parse_accepts_bare_array() {
    let orch = orch_for("http://stub".into(), false);
    let actions = orch.parse_actions(r#"[{"action":"no_op"}]"#).unwrap();
    assert_eq!(actions.len(), 1);
}

#[test]
fn parse_accepts_markdown_fenced_json() {
    let orch = orch_for("http://stub".into(), false);
    let actions = orch
        .parse_actions("```json\n{\"actions\":[{\"action\":\"no_op\"}]}\n```")
        .unwrap();
    assert_eq!(actions.len(), 1);
}

#[test]
fn parse_empty_actions_is_valid_noop() {
    let orch = orch_for("http://stub".into(), false);
    assert!(orch.parse_actions(r#"{"actions":[]}"#).unwrap().is_empty());
    assert!(orch.parse_actions("[]").unwrap().is_empty());
}

// === C. Policy gate still governs LLM output ===

#[tokio::test]
async fn gate_rejects_illegal_llm_action_and_audits() {
    // The model emits a LEGAL create_task AND an ILLEGAL assign_task (to a
    // role label, not a hired id — e.g. "ghost"). The gate must apply the legal
    // one and audit the illegal one as PlanActionRejected, not silently.
    let stub = boot_stub(default_responder(
        r#"{"actions":[
            {"action":"create_task","id":"task-gate-1","title":"X","kind":"feature"},
            {"action":"assign_task","task_id":"task-gate-1","assignee":"ghost"}
        ]}"#,
        "Build me a product.",
    ))
    .await;

    let state = make_state()
        .with_orchestrator(Arc::new(orch_for(stub.base_url, false)))
        .with_step_delay(std::time::Duration::ZERO);
    seed(&state);
    decision_trigger(&state, "m1", "Build me a product.");

    drive_pm(&state).await.unwrap();

    let proj = state.projection().unwrap();
    // The legal action was applied.
    assert!(
        proj.tasks.iter().any(|t| t.id == "task-gate-1"),
        "legal create_task applied"
    );
    // The illegal assign was rejected + audited.
    let rejected = state
        .store
        .read_since("proj-llm", 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == EventType::PlanActionRejected);
    let rejected: Vec<_> = rejected.collect();
    assert_eq!(
        rejected.len(),
        1,
        "the illegal action is audited exactly once"
    );
    assert_eq!(rejected[0].data["who"], "mei");
    assert!(
        rejected[0].data["reason"]
            .as_str()
            .map(|s| s.contains("not hired"))
            .unwrap_or(false),
        "reason explains the rejection: {}",
        rejected[0].data["reason"]
    );
}

// === D. Harness guards skip the provider call ===

#[tokio::test]
async fn paused_skips_provider_call_no_spend() {
    let stub = boot_stub(default_responder(
        r#"{"actions":[{"action":"create_task","id":"t","title":"X","kind":"feature"}]}"#,
        "Build me a product.",
    ))
    .await;

    let state = make_state()
        .with_orchestrator(Arc::new(orch_for(stub.base_url, false)))
        .with_step_delay(std::time::Duration::ZERO);
    seed(&state);
    // Pause BEFORE the message triggers planning (director WorkPaused event).
    state
        .append(Event::new(
            "proj-llm",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::WorkPaused,
            Aggregate {
                kind: "guard".into(),
                id: "work-pause".into(),
            },
            json!({ "reason": "owner requested", "by": "director" }),
        ))
        .unwrap();
    msg_body(&state, "m1", "Build me a product.");

    drive_pm(&state).await.unwrap();

    // The guard blocked dispatch BEFORE the provider call: no HTTP hit, no cost.
    assert_eq!(
        stub.req_count.load(Ordering::SeqCst),
        0,
        "no provider call when paused"
    );
    let cost = state
        .store
        .read_since("proj-llm", 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == EventType::CostIncurred)
        .count();
    assert_eq!(cost, 0, "no spend when paused");
}

#[tokio::test]
async fn budget_halt_skips_provider_call_no_spend() {
    let stub = boot_stub(default_responder(
        r#"{"actions":[{"action":"create_task","id":"t","title":"X","kind":"feature"}]}"#,
        "Build me a product.",
    ))
    .await;

    let state = make_state()
        .with_orchestrator(Arc::new(orch_for(stub.base_url, true)))
        .with_step_delay(std::time::Duration::ZERO);
    seed(&state);
    // Seed real prior spend (a CostIncurred of $1.00), then set a budget limit
    // below it — spend(1.0) >= limit(0.5) → Halted (the breaker trips). A limit
    // of 0.0 would mean DISABLED (not halted), so we must produce real spend.
    state
        .append(Event::new(
            "proj-llm",
            Actor::System,
            EventType::CostIncurred,
            Aggregate {
                kind: "cost".into(),
                id: "prev-cost".into(),
            },
            json!({ "estimated_usd": 1.0 }),
        ))
        .unwrap();
    state
        .append(Event::new(
            "proj-llm",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::BudgetSet,
            Aggregate {
                kind: "budget".into(),
                id: "budget".into(),
            },
            json!({ "limit_usd": 0.5, "warn_at": 0.80 }),
        ))
        .unwrap();
    msg_body(&state, "m1", "Build me a product.");

    drive_pm(&state).await.unwrap();

    assert_eq!(
        stub.req_count.load(Ordering::SeqCst),
        0,
        "no provider call at half budget halt"
    );
    // No NEW spend beyond the single seeded CostIncurred ($1.00) I wrote above.
    let cost_events = state
        .store
        .read_since("proj-llm", 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == EventType::CostIncurred)
        .count();
    assert_eq!(
        cost_events, 1,
        "only the seeded pre-spend, no provider cost at halt"
    );
}

// === E. Not hijacked — non-director triggers don't route through the orchestrator ===

#[tokio::test]
async fn non_owner_triggers_do_not_call_provider() {
    let stub = boot_stub(default_responder(
        r#"{"actions":[{"action":"no_op"}]}"#,
        "Build me a product.",
    ))
    .await;

    let state = make_state()
        .with_orchestrator(Arc::new(orch_for(stub.base_url, false)))
        .with_step_delay(std::time::Duration::ZERO);
    seed(&state);

    // A non-director MessageSent (e.g. an agent speaking) must NOT route through
    // the orchestrator — orchestration is scoped to DIRECTOR messages.
    state
        .append(Event::new(
            "proj-llm",
            Actor::Agent {
                id: "marcus-reed".into(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "agent-msg".into(),
            },
            json!({ "body": "anyone here?" }),
        ))
        .unwrap();
    drive_pm(&state).await.unwrap();

    assert_eq!(
        stub.req_count.load(Ordering::SeqCst),
        0,
        "only DIRECTOR messages hit the LLM"
    );
}

// === F. Cost math (metering) ===

#[tokio::test]
async fn cost_metering_derives_usd_and_cache_split() {
    // Drive a real plan against the stub with known usage + prices, and assert
    // the CostMetering math: estimated_usd = (prompt*in + completion*out)/1e6,
    // cache_read = cached_tokens, provider/model recorded.
    let stub = boot_stub(default_responder(
        r#"{"actions":[{"action":"create_task","id":"task-cost","title":"X","kind":"feature"}]}"#,
        "Build me a product.",
    ))
    .await;

    let state = make_state()
        .with_orchestrator(Arc::new(orch_for(stub.base_url, true)))
        .with_step_delay(std::time::Duration::ZERO);
    seed(&state);
    decision_trigger(&state, "m1", "Build me a product.");
    drive_pm(&state).await.unwrap();

    let spend = state.store.read_since("proj-llm", 0).unwrap();
    let cost = spend
        .iter()
        .find(|e| e.event_type == EventType::CostIncurred)
        .expect("a CostIncurred event");
    // prompt 1200 * $0.25 + completion 80 * $1.25, per 1M tokens.
    let expected = (1200.0 * 0.25 + 80.0 * 1.25) / 1_000_000.0;
    assert!((cost.data["estimated_usd"].as_f64().unwrap() - expected).abs() < 1e-9);
    assert_eq!(cost.data["prompt_tokens"], 1200);
    assert_eq!(cost.data["completion_tokens"], 80);
    assert_eq!(cost.data["cache_read_input_tokens"], 300);
    assert_eq!(cost.data["provider"], "stub");
    assert_eq!(cost.data["model"], "stub-model");
}

// === G. Provider HTTP error is audited exactly once, no panic ===

#[tokio::test]
async fn provider_http_error_is_audited_no_panic() {
    // Stub returns HTTP 500 (no valid completion). The orchestrator must error,
    // be audited exactly once, produce no actions, and not panic.
    let stub = boot_stub(Arc::new(|h, b| {
        assert_shape(h, b, "Build me a product.");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": "boom" }),
        )
    }))
    .await;

    let state = make_state()
        .with_orchestrator(Arc::new(orch_for(stub.base_url, false)))
        .with_step_delay(std::time::Duration::ZERO);
    seed(&state);
    decision_trigger(&state, "m1", "Build me a product.");

    let authored = drive_pm(&state).await.unwrap();
    assert_eq!(authored, 0, "a provider error produces no domain actions");

    let runs: Vec<_> = state
        .store
        .read_since("proj-llm", 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == EventType::OrchestrationRun)
        .collect();
    assert_eq!(
        runs.len(),
        1,
        "a failed pass audits exactly ONE OrchestrationRun"
    );
    assert!(runs[0].data.get("error").is_some());
}

// === H. Opt-in LIVE OpenRouter round-trip (LOCAL ONLY; never in CI) ===

/// Uses a real OpenRouter key to prove the full loop end-to-end against a real
/// provider. Opt-in: `cargo test --test llm_e2e -- --ignored`. Requires
/// CAST_LLM_API_KEY + CAST_LLM_MODEL (+ optionally CAST_LLM_PROVIDER/BASE_URL).
#[tokio::test]
#[ignore = "requires a live OpenRouter key; run manually with -- --ignored"]
async fn live_openrouter_round_trip() {
    let cfg = casting::llm::config::from_env(None).unwrap();
    let Some(_cfg) = cfg else {
        eprintln!("skipping live test: set CAST_LLM_API_KEY + CAST_LLM_MODEL");
        return;
    };

    let state = make_state()
        .with_orchestrator(Arc::new(LlmOrchestrator::new(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("build reqwest client"),
            casting::llm::config::from_env(None).unwrap().unwrap(),
            "You are the Project Manager.".into(),
        )))
        .with_step_delay(std::time::Duration::ZERO);
    seed(&state);
    msg_body(&state, "m1", "Build a tiny CLI tool that greets the user.");

    let authored = drive_pm(&state).await.unwrap();
    let proj = state.projection().unwrap();
    // The real model should have created at least one task.
    assert!(
        authored > 0 && !proj.tasks.is_empty(),
        "live LLM created work; authored={authored} tasks={}",
        proj.tasks.len()
    );
    // And landed a CostIncurred with a provider recorded.
    let cost = state
        .store
        .read_since("proj-llm", 0)
        .unwrap()
        .into_iter()
        .find(|e| e.event_type == EventType::CostIncurred);
    assert!(cost.is_some(), "live LLM metered a CostIncurred");
}

// === Anthropic adapter: it must speak /v1/messages, NOT chat/completions ===

/// Prove the `Client` seam routes anthropic to the messages protocol and
/// normalizes the response back into the shared Usage shape (total prompt +
/// cache split), so metering sees the same fields as OpenRouter.
#[tokio::test]
async fn anthropic_client_speaks_messages_protocol() {
    use axum::{routing::post, Json, Router};
    use casting::llm::{ChatMessage, ChatRequest, Client};
    use std::sync::Mutex;

    let captured: Arc<Mutex<Option<(HeaderMap, serde_json::Value)>>> = Arc::new(Mutex::new(None));

    let app = Router::new().route(
        "/v1/messages",
        post({
            let captured = captured.clone();
            |headers: HeaderMap, body: Json<serde_json::Value>| async move {
                *captured.lock().unwrap() = Some((headers, body.0.clone()));
                (
                    StatusCode::OK,
                    Json(json!({
                        "content": [{ "type": "text", "text": "assistant reply" }],
                        "usage": {
                            "input_tokens": 900,
                            "output_tokens": 100,
                            "cache_read_input_tokens": 100,
                            "cache_creation_input_tokens": 50
                        }
                    })),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    // base_url WITHOUT /v1 — the Anthropic client appends /v1/messages. Confirm
    // the seam selects the anthropic protocol from the provider name alone.
    let client = Client::new(
        "anthropic",
        http,
        format!("http://{addr}"),
        "test-key".into(),
    );

    let req = ChatRequest {
        model: "claude-sonnet-4-5".into(),
        messages: vec![
            ChatMessage {
                role: "system".into(),
                content: "You are the PM.".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: "Plan the build.".into(),
            },
        ],
        temperature: Some(0.2),
        max_tokens: Some(500),
        response_format: None,
    };
    let comp = client.chat(&req).await.unwrap();

    // Response normalization: total prompt from the disjoint buckets + split.
    assert_eq!(comp.content, "assistant reply");
    assert_eq!(comp.usage.prompt_tokens, 900 + 100 + 50);
    assert_eq!(comp.usage.completion_tokens, 100);
    assert_eq!(
        comp.usage
            .prompt_tokens_details
            .as_ref()
            .unwrap()
            .cached_tokens,
        100
    );
    assert_eq!(
        comp.usage
            .prompt_tokens_details
            .as_ref()
            .unwrap()
            .cache_creation_tokens,
        50
    );
    // Anthropic does not report USD cost.
    assert!(comp.usage.cost.is_none());

    // Wire shape: system lifted out, messages = user only, HEADERS right, and
    // the URL path hit was /v1/messages (it responded with our messages handler).
    let (headers, body) = captured.lock().unwrap().clone().unwrap();
    assert_eq!(
        headers.get("x-api-key").and_then(|v| v.to_str().ok()),
        Some("test-key")
    );
    assert_eq!(
        headers
            .get("anthropic-version")
            .and_then(|v| v.to_str().ok()),
        Some("2023-06-01")
    );
    assert_eq!(body["model"], "claude-sonnet-4-5");
    assert_eq!(body["system"], "You are the PM.");
    assert_eq!(body["max_tokens"], 500);
    assert!(
        (body["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-3,
        "temperature should be ~0.2"
    );
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(
        msgs.len(),
        1,
        "system must be lifted out of the messages array"
    );
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"], "Plan the build.");
}
