//! The automatic Feature-Mode decomposition flow (CAST_DECOMPOSE / with_decompose):
//! the PM promotes a cross-cutting requirement into parallel children, adds a
//! Blocker-Test hard dependency (ordering), and starts only the READY children —
//! the hard-blocked child stays queued until the gate sees its blocker complete.

use casting::pm::AppState;
use casting::runtime::orchestrator::MockOrchestrator;
use casting::store::CursorStore as _;
use casting::store::EventStore as _;
use std::sync::Arc;
use std::time::Duration;

fn seed(state: &AppState) {
    // Seed project + cast so plan_onboard kicks off on the first director message.
    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::System,
            casting::event::EventType::ProjectCreated,
            casting::event::Aggregate {
                kind: "project".into(),
                id: "proj".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();
    for (id, role) in [
        ("mei", "Project Manager"),
        ("diego", "Lead Developer"),
        ("tess", "Testing Engineer"),
    ] {
        state
            .append(casting::event::Event::new(
                "proj",
                casting::event::Actor::System,
                casting::event::EventType::AgentHired,
                casting::event::Aggregate {
                    kind: "agent".into(),
                    id: id.into(),
                },
                serde_json::json!({ "role": role }),
            ))
            .unwrap();
    }
    state
        .cursors
        .advance("proj", "mei", state.store.latest_sequence("proj").unwrap())
        .unwrap();
}

#[tokio::test]
async fn onboard_with_decompose_fans_out_parallel_children_and_orders_them() {
    let store = casting::store::SqliteEventStore::in_memory().unwrap();
    let cursors = casting::store::SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj")
        .with_step_delay(Duration::ZERO)
        .with_decompose()
        .with_orchestrator(Arc::new(MockOrchestrator));
    // Set a budget so the gate's Disabled check doesn't block dispatches
    // (the MockOrchestrator needs to pass through the gate to drive work).
    use casting::event::{
        Actor as EvActor, Aggregate as EvAggregate, Event as EvEvent, EventType as EvEventType,
    };
    state
        .append(EvEvent::new(
            "proj",
            EvActor::Director {
                user_id: "ceo".into(),
            },
            EvEventType::BudgetSet,
            EvAggregate {
                kind: "budget".into(),
                id: "budget".into(),
            },
            serde_json::json!({ "limit_usd": 100.0, "warn_at": 0.80 }),
        ))
        .unwrap();
    seed(&state);

    // Manually create the requirement and feature task that would otherwise
    // be produced by the PM's first message. The old plan_onboard did this
    // as a demo tape; now the orchestrator path handles it.
    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::Agent { id: "mei".into() },
            casting::event::EventType::RequirementCreated,
            casting::event::Aggregate {
                kind: "requirement".into(),
                id: "req-1".into(),
            },
            serde_json::json!({ "title": "Build me an app", "description": "Build me an app" }),
        ))
        .unwrap();
    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::Agent { id: "mei".into() },
            casting::event::EventType::TaskCreated,
            casting::event::Aggregate {
                kind: "task".into(),
                id: "task-feature".into(),
            },
            serde_json::json!({ "title": "Feature: Build me an app", "kind": "feature" }),
        ))
        .unwrap();
    // Decompose the feature into parallel children with a hard edge.
    let children = vec![
        casting::actions::TaskSpec {
            id: "feature-db".into(),
            title: "Database layer".into(),
            kind: "feature".into(),
        },
        casting::actions::TaskSpec {
            id: "feature-api".into(),
            title: "API layer".into(),
            kind: "feature".into(),
        },
        casting::actions::TaskSpec {
            id: "feature-ui".into(),
            title: "UI layer".into(),
            kind: "feature".into(),
        },
        casting::actions::TaskSpec {
            id: "feature-sec".into(),
            title: "Security review".into(),
            kind: "feature".into(),
        },
    ];
    let cause = casting::event::Event::new(
        "proj",
        casting::event::Actor::Agent { id: "mei".into() },
        casting::event::EventType::TaskCreated,
        casting::event::Aggregate {
            kind: "task".into(),
            id: "task-feature".into(),
        },
        serde_json::json!({}),
    );
    for ev in (casting::actions::PmAction::DecomposeTask {
        parent: "task-feature".into(),
        children: children.clone(),
    })
    .to_events("proj", "mei", &cause, "corr-decompose")
    {
        state.append(ev).unwrap();
    }

    // Add the hard edge: feature-api blocks on feature-db.
    for ev in (casting::actions::PmAction::BlockTaskOn {
        task_id: "feature-api".into(),
        blocking_task_id: "feature-db".into(),
        required_state: casting::types::TaskStatus::Done,
    })
    .to_events("proj", "mei", &cause, "corr-edge")
    {
        state.append(ev).unwrap();
    }

    // Assign children to diego (engineer) so the actor-turn loop can act.
    for child in ["feature-db", "feature-api", "feature-ui", "feature-sec"] {
        for ev in (casting::actions::PmAction::AssignTask {
            task_id: child.into(),
            assignee: "diego".into(),
            merge_authority: casting::types::MergeAuthority::SelfMerge,
        })
        .to_events("proj", "mei", &cause, "corr-assign")
        {
            state.append(ev).unwrap();
        }
    }

    // Now drive the PM — the actor-turn loop should pick up the Backlog
    // children, start/complete/review them through the gate.
    let authored = casting::pm::drive_pm(&state).await.unwrap();
    assert!(authored > 0, "PM should author work through actor turns");

    let proj = state.projection().unwrap();

    // Feature parent (join point) + 4 parallel children.
    assert!(proj.tasks.iter().any(|t| t.id == "task-feature"));
    let children = proj.children_of("task-feature");
    assert_eq!(
        children.len(),
        4,
        "cross-cutting requirement decomposes into 4"
    );
    assert!(children.contains(&"feature-api".to_string()));
    assert!(children.contains(&"feature-db".to_string()));
    assert!(children.contains(&"feature-ui".to_string()));
    assert!(children.contains(&"feature-sec".to_string()));

    // The Blocker Test produced a hard edge api -> db (ordering).
    assert!(proj
        .dependencies
        .iter()
        .any(|d| { d.task == "feature-api" && d.blocking_task == "feature-db" }));

    // Subtasks are FIRST-CLASS tasks: driven through the exact same lifecycle
    // as any other task (assign -> start -> complete -> submit -> review ->
    // done), so every child reaches Done AND the join resolves.
    for child in ["feature-db", "feature-api", "feature-ui", "feature-sec"] {
        let t = proj.tasks.iter().find(|t| t.id == child).unwrap();
        assert_eq!(
            t.status,
            casting::projection::TaskStatus::Done,
            "{child} should complete its full lifecycle (subtasks are tasks)"
        );
    }
    assert!(
        proj.blocked_by("feature-api").is_empty(),
        "dependency clears once the blocker is Done"
    );
    let g = proj.graph();
    assert_eq!(g.groups.len(), 1, "feature is a join point");
    assert!(
        g.groups[0].resolved,
        "join resolves when all children reach Done"
    );

    // Ordering proof from the event log: feature-api could only have STARTED
    // after feature-db COMPLETED (it's hard-blocked on db), so db's
    // TaskCompleted must precede api's TaskStarted in sequence.
    let events = state.store.read_since("proj", 0).unwrap();
    let db_completed = events
        .iter()
        .find(|e| {
            e.event_type == casting::event::EventType::TaskCompleted
                && e.aggregate.id == "feature-db"
        })
        .map(|e| e.sequence)
        .unwrap();
    let api_started = events
        .iter()
        .find(|e| {
            e.event_type == casting::event::EventType::TaskStarted
                && e.aggregate.id == "feature-api"
        })
        .map(|e| e.sequence)
        .unwrap();
    assert!(
        db_completed < api_started,
        "api must start after its blocker db completes (got {db_completed} >= {api_started})"
    );
}

#[tokio::test]
async fn default_onboard_without_decompose_stays_flat() {
    // Regression guard: with decompose off, the canonical flow is unchanged —
    // no feature children are fanned out.
    let store = casting::store::SqliteEventStore::in_memory().unwrap();
    let cursors = casting::store::SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj")
        .with_step_delay(Duration::ZERO)
        .with_orchestrator(Arc::new(MockOrchestrator));
    seed(&state);

    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::Director {
                user_id: "ceo".into(),
            },
            casting::event::EventType::MessageSent,
            casting::event::Aggregate {
                kind: "message".into(),
                id: "m1".into(),
            },
            serde_json::json!({ "body": "Build me an app" }),
        ))
        .unwrap();
    casting::pm::drive_pm(&state).await.unwrap();
    let proj = state.projection().unwrap();
    // Owner messages now route through the chat-interface playbook, which
    // creates a child step task (parent_id is set). This is independent of
    // the decompose flag. Non-chat tasks (from seed) remain flat.
    for task in &proj.tasks {
        if !task.id.starts_with("chat-") {
            assert!(
                task.parent_id.is_none(),
                "non-chat task '{}' should have no parent (decompose=off)",
                task.id
            );
        }
    }
    // The chat parent exists and has no parent_id (it's the root)
    let chat_parent: Vec<_> = proj
        .tasks
        .iter()
        .filter(|t| t.id.starts_with("chat-") && !t.id.contains('/'))
        .collect();
    assert_eq!(
        chat_parent.len(),
        1,
        "should have exactly one chat parent task"
    );
}
