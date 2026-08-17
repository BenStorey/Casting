//! Tests that the executor seam (run_side_effect / workspace_activity_for) now
//! carries the real workspace side effects through the SAME guards as any other
//! side effect — closing the architecture-review gap where inline worktree/git
//! hooks bypassed the pause/budget/secret gates.

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::runtime::executor::{run_side_effect, workspace_activity_for, Activity, ActivityKind};
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
use std::sync::atomic::{AtomicUsize, Ordering};

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj-se");
    state
        .append(Event::new(
            "proj-se",
            Actor::System,
            EventType::ProjectCreated,
            Aggregate {
                kind: "project".into(),
                id: "proj-se".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    state
}

/// A runner that counts invocations — lets us assert the guard stopped it
/// BEFORE any physical work ran.
#[derive(Default)]
struct CountingRunner(AtomicUsize);

impl casting::runtime::executor::ActivityRunner for CountingRunner {
    fn run(
        &self,
        _activity: &Activity,
    ) -> anyhow::Result<casting::runtime::executor::ActivityResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(casting::runtime::executor::ActivityResult::default())
    }
}

fn provision_activity() -> Activity {
    Activity {
        id: "worktree-t-1".into(),
        target_id: "t-1".into(),
        kind: ActivityKind::ProvisionWorktree {
            task_id: "t-1".into(),
            assignee: "lead-programmer".into(),
            slug: "".into(),
            slot: 0,
            port: 8081,
        },
    }
}

fn pause(state: &AppState) {
    state
        .append(Event::new(
            "proj-se",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::WorkPaused,
            Aggregate {
                kind: "guard".into(),
                id: "work-pause".into(),
            },
            serde_json::json!({ "reason": "manual", "by": "director" }),
        ))
        .unwrap();
}

fn halt_budget(state: &AppState) {
    state
        .append(Event::new(
            "proj-se",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::BudgetSet,
            Aggregate {
                kind: "budget".into(),
                id: "budget".into(),
            },
            serde_json::json!({ "limit_usd": 1.0, "warn_at": 0.8 }),
        ))
        .unwrap();
    // spend $2 > $1 limit.
    state
        .append(Event::new(
            "proj-se",
            Actor::System,
            EventType::CostIncurred,
            Aggregate {
                kind: "cost".into(),
                id: "cost-1".into(),
            },
            serde_json::json!({
                "agent_id": "mei", "prompt_tokens": 0, "completion_tokens": 0,
                "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0,
                "latency_ms": 0, "estimated_usd": 2.0,
            }),
        ))
        .unwrap();
}

// --- event → activity mapping (the single, central place) ---

#[test]
fn workspace_activity_for_maps_provision_and_commit() {
    let prov = workspace_activity_for(&Event::new(
        "proj-se",
        Actor::System,
        EventType::WorktreeProvisioned,
        Aggregate {
            kind: "task".into(),
            id: "t-9".into(),
        },
        serde_json::json!({ "task_id": "t-9", "port": 8082 }),
    ))
    .expect("provision maps");
    assert!(matches!(
        prov.kind,
        ActivityKind::ProvisionWorktree { port: 8082, .. }
    ));

    let commit = workspace_activity_for(&Event::new(
        "proj-se",
        Actor::System,
        EventType::CommitRequested,
        Aggregate {
            kind: "task".into(),
            id: "t-9".into(),
        },
        serde_json::json!({ "message": "wip" }),
    ))
    .expect("commit maps");
    assert!(matches!(
        commit.kind,
        ActivityKind::CommitWorktree { message, .. } if message == "wip"
    ));

    // Unrelated events carry no workspace side effect.
    assert!(workspace_activity_for(&Event::new(
        "proj-se",
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "m".into(),
        },
        serde_json::json!({})
    ))
    .is_none());
}

// --- the guard now reaches the workspace path ---

#[test]
fn run_side_effect_refuses_when_paused() {
    let state = make_state();
    pause(&state);
    let runner = CountingRunner::default();
    let err = run_side_effect(&state, &runner, Actor::System, &provision_activity()).unwrap_err();
    assert!(err.to_string().contains("paused"));
    assert_eq!(
        runner.0.load(Ordering::SeqCst),
        0,
        "runner must not be called when paused"
    );
}

#[test]
fn run_side_effect_refuses_when_budget_halted() {
    let state = make_state();
    halt_budget(&state);
    let runner = CountingRunner::default();
    let err = run_side_effect(&state, &runner, Actor::System, &provision_activity()).unwrap_err();
    assert!(err.to_string().contains("budget"));
    assert_eq!(
        runner.0.load(Ordering::SeqCst),
        0,
        "runner must not be called when halted"
    );
}

#[test]
fn run_side_effect_runs_when_unguarded() {
    let state = make_state();
    // Set a budget so the gate's Disabled check doesn't interfere — the
    // purpose of this test is to verify the run path when nothing is blocked.
    state
        .append(Event::new(
            "proj-se",
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
    let runner = CountingRunner::default();
    run_side_effect(&state, &runner, Actor::System, &provision_activity()).unwrap();
    assert_eq!(runner.0.load(Ordering::SeqCst), 1);
    // The domain-side-effect path now records ActivityScheduled + ActivityCompleted
    // lifecycle events (C3 fix), so crash recovery + watchdog RetryLoop work.
    let events = state.store.read_since("proj-se", 0).unwrap();
    assert!(events
        .iter()
        .any(|e| e.event_type == EventType::ActivityScheduled));
    assert!(events
        .iter()
        .any(|e| e.event_type == EventType::ActivityCompleted));
}

// --- guard's budget status is projected (sanity that halt_budget folded) ---
#[test]
fn halted_budget_folds() {
    let state = make_state();
    halt_budget(&state);
    let p = Projection::build(&state.store, "proj-se").unwrap();
    assert!(p.budget.is_some());
    assert!(casting::pm::guard::llm_dispatch_allowed(&p).is_err());
}
