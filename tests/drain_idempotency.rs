//! Idempotent PM drain — a mid-drain failure must not re-emit duplicate domain
//! events on re-entry.
//!
//! The PM's `drain` reads events since its durable cursor, calls `respond()`
//! (which plans + appends), THEN advances the cursor. If `respond()`/the
//! append path fails mid-way (an error propagates via `?`), the cursor is NOT
//! advanced; the next wake re-reads from the old cursor and re-plans the SAME
//! causes. `run_planned` now skips a real-entity DOMAIN event that was already
//! applied for the same planning cause (same event_type + aggregate.id +
//! correlation_id), while audit/telemetry records (PlanActionRejected,
//! OrchestrationRun, ...) keep appending as-is so the audit trail stays intact.
//!
//! These tests simulate the failed first drain by persisting placeholder events
//! for a planning cause (appended by a first drain that errored BEFORE the
//! cursor advanced — so the cursor still points before the cause), then drive
//! the drain again and assert no duplicate requirement/task/decision is created
//! while the audit events are all still present.

use casting::actions::PmAction;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::store::EventStore;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
use std::time::Duration;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-idem").with_step_delay(Duration::ZERO)
}

fn owner_message(body: &str) -> Event {
    Event::new(
        "proj-idem",
        Actor::Owner,
        EventType::MessageSent,
        Aggregate {
            kind: "message".into(),
            id: "msg-owner".into(),
        },
        serde_json::json!({ "to": "pm", "body": body }),
    )
}

fn count(state: &AppState, pred: impl Fn(&Event) -> bool) -> usize {
    state
        .store
        .read_since("proj-idem", 0)
        .unwrap()
        .iter()
        .filter(|e| pred(e))
        .count()
}

/// Mid-drain failure (cursor NEVER advanced) → the next drain re-reads the SAME
/// owner message and re-plans the SAME onboard. The events the failed first
/// drain already persisted (a real-entity TaskCreated + TaskAssigned for
/// task-design, with the run-{seq} correlation) must NOT be re-emitted, while
/// the requirement/decision are created exactly once.
#[tokio::test]
async fn failed_drain_rereads_but_does_not_duplicate_domain_events() {
    let state = make_state();

    // The cause: one owner message. Its run-{seq} correlation is the unique
    // per-cause planning key the idempotency guard keys on.
    let cause = state.append(owner_message("Build a todo app")).unwrap();
    let corr = format!("run-{}", cause.sequence);
    assert_eq!(corr, "run-1");

    // (a) Simulate a FAILED first drain: the PM's onboard plan got as far as
    //     creating+assigning task-design, and `respond()` errored BEFORE the
    //     cursor advanced. The cursor is still before the cause.
    let task_created = PmAction::CreateTask {
        id: "task-design".into(),
        title: "Design Build a todo app".into(),
        kind: "feature".into(),
    }
    .to_events("proj-idem", "pm", &cause, &corr);
    for e in task_created {
        state.append(e).unwrap();
    }
    let task_assigned = PmAction::AssignTask {
        task_id: "task-design".into(),
        assignee: "marcus-reed".into(),
        merge_authority: casting::types::MergeAuthority::SelfMerge,
    }
    .to_events("proj-idem", "pm", &cause, &corr);
    for e in task_assigned {
        state.append(e).unwrap();
    }
    // Cursor is deliberately NOT advanced — it still points before `cause`.

    // (b) Run the drain again: re-reads the same cause and re-plans onboard.
    casting::pm::drive_pm(&state).await.unwrap();

    // NO duplicate requirement/task/decision is created.
    assert_eq!(
        count(&state, |e| e.event_type == EventType::RequirementCreated),
        1,
        "re-entry must not create a second requirement"
    );
    assert_eq!(
        count(&state, |e| {
            e.event_type == EventType::TaskCreated && e.aggregate.id == "task-design"
        }),
        1,
        "re-entry must not re-emit the already-applied TaskCreated"
    );
    assert_eq!(
        count(&state, |e| {
            e.event_type == EventType::TaskAssigned && e.aggregate.id == "task-design"
        }),
        1,
        "re-entry must not re-emit the already-applied TaskAssigned \
         (the gate allows a re-assign, so ONLY the idempotency guard stops it)"
    );
    assert_eq!(
        count(&state, |e| {
            e.event_type == EventType::DecisionProposed && e.aggregate.id == "decision-db"
        }),
        1,
        "re-entry must not duplicate the Database decision"
    );

    // The projected state agrees: one requirement, one task-design, no dupes.
    let proj = Projection::build(&state.store, "proj-idem").unwrap();
    assert_eq!(proj.requirements.len(), 1);
    assert_eq!(
        proj.tasks.iter().filter(|t| t.id == "task-design").count(),
        1,
        "exactly one task-design in the projection"
    );
    assert_eq!(
        proj.decisions
            .iter()
            .filter(|d| d.id == "decision-db")
            .count(),
        1,
        "exactly one Database decision in the projection"
    );
}

/// Audit / telemetry records deliberately share ONE aggregate id AND the
/// run-{seq} correlation per planning pass. A naive dedup by (event_type,
/// aggregate.id, correlation) would collapse distinct rejections into one and
/// break the audit trail. Our guard must NOT dedup them — all audit events stay
/// present (un-collapsed) through a re-entry.
#[tokio::test]
async fn reentry_preserves_audit_events_unchanged() {
    let state = make_state();

    let cause = state.append(owner_message("Build a todo app")).unwrap();
    let corr = format!("run-{}", cause.sequence);

    // Seed the failed first drain's audit records: MULTIPLE distinct
    // PlanActionRejected + an OrchestrationRun, ALL sharing the shared "plan"
    // aggregate id "run-1" and correlation "run-1" (exactly how the real audit
    // trail is shaped). These must survive re-entry intact.
    for (i, reason) in ["TaskNotFound", "PolicyRefused"].iter().enumerate() {
        state
            .append(Event::new(
                "proj-idem",
                Actor::System,
                EventType::PlanActionRejected,
                Aggregate {
                    kind: "plan".into(),
                    id: corr.clone(),
                },
                serde_json::json!({
                    "who": "pm",
                    "action": format!("action-{i}"),
                    "reason": reason,
                    "correlation": corr,
                }),
            ))
            .unwrap();
    }
    state
        .append(Event::new(
            "proj-idem",
            Actor::System,
            EventType::OrchestrationRun,
            Aggregate {
                kind: "plan".into(),
                id: corr.clone(),
            },
            serde_json::json!({
                "trigger": "MessageSent",
                "actor": "pm",
                "correlation": corr,
                "context_summary": "objective=Build a todo app",
                "planned": ["pm -> create_task"],
                "metered": false,
            }),
        ))
        .unwrap();

    // Re-run the drain.
    casting::pm::drive_pm(&state).await.unwrap();

    // The audit trail is UNCOLLAPSED: all distinct AuditPlanActionRejected
    // records sharing the same correlation/aggregate still coexist.
    let all = state.store.read_since("proj-idem", 0).unwrap();
    let rejs: Vec<&Event> = all
        .iter()
        .filter(|e| e.event_type == EventType::PlanActionRejected)
        .collect();
    assert!(
        rejs.len() >= 2,
        "audit rejections must not be collapsed; got {}",
        rejs.len()
    );
    let reasons: std::collections::BTreeSet<&str> = rejs
        .iter()
        .filter_map(|e| e.data.get("reason").and_then(|r| r.as_str()))
        .collect();
    assert!(
        reasons.contains("TaskNotFound") && reasons.contains("PolicyRefused"),
        "all distinct rejection reasons preserved: {reasons:?}"
    );
    assert_eq!(
        count(&state, |e| e.event_type == EventType::OrchestrationRun),
        1,
        "OrchestrationRun audit record preserved"
    );

    // And the domain side still shows no duplicates through the re-entry.
    assert_eq!(
        count(&state, |e| e.event_type == EventType::RequirementCreated),
        1,
        "re-entry must not create a second requirement"
    );
    assert_eq!(
        count(&state, |e| {
            e.event_type == EventType::TaskCreated && e.aggregate.id == "task-design"
        }),
        1,
        "re-entry must not re-emit the already-applied TaskCreated"
    );
}
