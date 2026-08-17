//! Tests for the harness guards — the hard circuit breaker + pause rails
//! (docs/plans/2026-08-13_harness-guards.md).
//!
//! The PM *optimizes*; the guard *refuses*. These tests prove the guard rails
//! hold deterministically and OUTSIDE the PM's control: budget is derived from
//! spend (never decreases, not resumable), pause is a resumable director/watchdog
//! flag, and every LLM/side-effect dispatch point consults the gate.

use casting::actions::{validate, PmAction, PolicyError};
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::guard::{self, BudgetStatus};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::runtime::executor::{execute, Activity, ActivityKind, NoopRunner};
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj-guard");
    // Seed the project (matches main.rs startup) so the projection folds cleanly.
    state
        .append(Event::new(
            "proj-guard",
            Actor::System,
            EventType::ProjectCreated,
            Aggregate {
                kind: "project".into(),
                id: "proj-guard".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    state
}

/// Append a `CostIncurred` (the reducer reads `estimated_usd`) so spend grows.
fn incur(state: &AppState, usd: f64) {
    state
        .append(Event::new(
            "proj-guard",
            Actor::System,
            EventType::CostIncurred,
            Aggregate {
                kind: "cost".into(),
                id: format!("cost-{}", uuid_like()),
            },
            serde_json::json!({
                "agent_id": "pm",
                "model_tier": "flash",
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0,
                "latency_ms": 0,
                "estimated_usd": usd,
            }),
        ))
        .unwrap();
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{}", N.fetch_add(1, Ordering::SeqCst))
}

fn set_budget(state: &AppState, limit_usd: f64, warn_at: f64) {
    state
        .append(Event::new(
            "proj-guard",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::BudgetSet,
            Aggregate {
                kind: "budget".into(),
                id: "budget".into(),
            },
            serde_json::json!({ "limit_usd": limit_usd, "warn_at": warn_at }),
        ))
        .unwrap();
}

fn proj(state: &AppState) -> Projection {
    Projection::build(&state.store, "proj-guard").unwrap()
}

// --- Budget folding + phases ---

#[test]
fn budget_set_folds_into_projection_with_warn_fraction() {
    let state = make_state();
    set_budget(&state, 100.0, 0.5);
    let p = proj(&state);
    let b = p.budget.expect("budget set");
    assert_eq!(b.limit_usd, 100.0);
    assert_eq!(b.warn_at, 0.5);
}

#[test]
fn budget_status_phases_follow_spend_fraction() {
    let state = make_state();
    // No budget -> Disabled (the gate lets everything through).
    assert_eq!(guard::budget_status(&proj(&state)), BudgetStatus::Disabled);

    set_budget(&state, 100.0, 0.80);
    // 1 / 100 -> Ok
    incur(&state, 1.0);
    assert_eq!(guard::budget_status(&proj(&state)), BudgetStatus::Ok);

    // 85 / 100 -> Warn (>= 0.80)
    incur(&state, 84.0);
    assert!(matches!(
        guard::budget_status(&proj(&state)),
        BudgetStatus::Warn { .. }
    ));

    // 120 / 100 -> Halted
    incur(&state, 35.0);
    assert!(matches!(
        guard::budget_status(&proj(&state)),
        BudgetStatus::Halted { .. }
    ));
}

// --- The gate (llm_dispatch_allowed) ---

#[test]
fn gate_blocks_when_budget_halted_and_warns_at_threshold() {
    let state = make_state();
    set_budget(&state, 100.0, 0.80);
    incur(&state, 5.0); // 5%
    assert_eq!(guard::budget_status(&proj(&state)), BudgetStatus::Ok);
    assert!(guard::llm_dispatch_allowed(&proj(&state)).is_ok());

    incur(&state, 90.0); // 95% -> Warn, still allowed
    assert!(matches!(
        guard::budget_status(&proj(&state)),
        BudgetStatus::Warn { .. }
    ));
    assert!(
        guard::llm_dispatch_allowed(&proj(&state)).is_ok(),
        "warn threshold still dispatches"
    );

    incur(&state, 20.0); // 115% -> Halted, refused
    assert!(matches!(
        guard::budget_status(&proj(&state)),
        BudgetStatus::Halted { .. }
    ));
    assert!(
        guard::llm_dispatch_allowed(&proj(&state)).is_err(),
        "hard breaker must refuse dispatch"
    );
}

#[test]
fn budget_halt_is_not_resumable_by_work_resume() {
    let state = make_state();
    set_budget(&state, 10.0, 0.80);
    incur(&state, 50.0); // way over
    assert!(guard::llm_dispatch_allowed(&proj(&state)).is_err());

    // Even after a resume, spend hasn't gone down -> still refused.
    state
        .append(Event::new(
            "proj-guard",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::WorkResumed,
            Aggregate {
                kind: "guard".into(),
                id: "work-pause".into(),
            },
            serde_json::json!({ "by": "director" }),
        ))
        .unwrap();
    assert!(
        guard::llm_dispatch_allowed(&proj(&state)).is_err(),
        "budget halt is derived from spend and cannot be resumed away"
    );

    // Raising the limit un-halts it.
    set_budget(&state, 500.0, 0.80);
    assert!(guard::llm_dispatch_allowed(&proj(&state)).is_ok());
}

// --- Pause (director / watchdog), resumable ---

#[test]
fn pause_and_resume_block_and_unblock_dispatch() {
    let state = make_state();
    // Set a budget so the gate's Disabled check doesn't interfere with the
    // pause test — budget is orthogonal to pause.
    set_budget(&state, 100.0, 0.80);
    assert!(guard::llm_dispatch_allowed(&proj(&state)).is_ok());

    // Owner pauses.
    for ev in (PmAction::PauseWork {
        reason: "manual hold".into(),
    })
    .to_events("proj-guard", "director", &cause(), "guard")
    {
        state.append(ev).unwrap();
    }
    let p = proj(&state);
    assert!(guard::is_paused(&p));
    let reason = guard::llm_dispatch_allowed(&p).unwrap_err();
    assert!(reason.contains("manual hold"));

    // Owner resumes -> clears.
    for ev in (PmAction::ResumeWork).to_events("proj-guard", "director", &cause(), "guard") {
        state.append(ev).unwrap();
    }
    let p = proj(&state);
    assert!(!guard::is_paused(&p));
    assert!(guard::llm_dispatch_allowed(&p).is_ok());
}

fn cause() -> Event {
    Event::new(
        "proj-guard",
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "msg-x".into(),
        },
        serde_json::json!({"body": ""}),
    )
}

// --- Authority: guards are OUTSIDE the PM ---

#[test]
fn only_owner_can_set_budget_or_resume() {
    let state = make_state();
    let p = proj(&state);

    assert!(matches!(
        validate(
            &PmAction::SetBudget {
                limit_usd: 10.0,
                warn_at: None
            },
            "pm",
            &p,
            None
        ),
        Err(PolicyError::GuardAuthority(_))
    ));
    assert!(matches!(
        validate(&PmAction::ResumeWork, "marcus-reed", &p, None),
        Err(PolicyError::GuardAuthority(_))
    ));
    // PM may not pause work either.
    assert!(matches!(
        validate(&PmAction::PauseWork { reason: "r".into() }, "pm", &p, None),
        Err(PolicyError::GuardAuthority(_))
    ));
    // Owner may do all three; system may pause (watchdog) but not set budget.
    assert!(validate(
        &PmAction::SetBudget {
            limit_usd: 10.0,
            warn_at: None
        },
        "director",
        &p,
        None
    )
    .is_ok());
    assert!(validate(
        &PmAction::PauseWork { reason: "r".into() },
        "system",
        &p,
        None
    )
    .is_ok());
    assert!(matches!(
        validate(
            &PmAction::SetBudget {
                limit_usd: 10.0,
                warn_at: None
            },
            "system",
            &p,
            None
        ),
        Err(PolicyError::GuardAuthority(_))
    ));
}

// --- Executor refuses side-effecting work while guarded (fail-closed) ---

#[test]
fn executor_refuses_side_effect_when_budget_halted() {
    let state = make_state();
    set_budget(&state, 5.0, 0.80);
    incur(&state, 20.0); // halted
    let runner = NoopRunner;

    // A side-effecting activity is refused.
    let activity = Activity {
        id: "t-1-shell".into(),
        target_id: "t-1".into(),
        kind: ActivityKind::Shell {
            cmd: "echo hi".into(),
        },
    };
    let err = execute(&state, &runner, Actor::System, &activity).unwrap_err();
    assert!(err.to_string().contains("guard blocked"));

    // ... and an ActivityFailed lands so it won't re-dispatch later.
    let events = state.store.read_since("proj-guard", 0).unwrap();
    assert!(events
        .iter()
        .any(|e| e.event_type == EventType::ActivityFailed
            && e.data.get("id").and_then(|v| v.as_str()) == Some("t-1-shell")));

    // Inline (derived) work is always allowed.
    let inline = Activity {
        id: "t-2-inline".into(),
        target_id: "t-2".into(),
        kind: ActivityKind::Inline,
    };
    assert!(execute(&state, &runner, Actor::System, &inline).is_ok());
}
