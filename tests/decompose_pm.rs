//! The automatic Feature-Mode decomposition flow (CAST_DECOMPOSE / with_decompose):
//! the PM promotes a cross-cutting requirement into parallel children, adds a
//! Blocker-Test hard dependency (ordering), and starts only the READY children —
//! the hard-blocked child stays queued until the gate sees its blocker complete.

use casting::cursor::CursorStore as _;
use casting::pm::AppState;
use casting::store::EventStore as _;
use std::time::Duration;

fn seed(state: &AppState) {
    // Seed project + cast so plan_onboard kicks off on the first owner message.
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
        ("pm", "Project Manager"),
        ("marcus-reed", "Engineer"),
        ("maya-patel", "QA"),
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
        .advance("proj", "pm", state.store.latest_sequence("proj").unwrap())
        .unwrap();
}

#[tokio::test]
async fn onboard_with_decompose_fans_out_parallel_children_and_orders_them() {
    let store = casting::sqlite_store::SqliteEventStore::in_memory().unwrap();
    let cursors = casting::cursor::SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj")
        .with_step_delay(Duration::ZERO)
        .with_decompose();
    seed(&state);

    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::Owner,
            casting::event::EventType::MessageSent,
            casting::event::Aggregate {
                kind: "message".into(),
                id: "m1".into(),
            },
            serde_json::json!({ "body": "Build me an app" }),
        ))
        .unwrap();

    let authored = casting::pm::drive_pm(&state).await.unwrap();
    assert!(authored > 0, "PM should author onboarding work");

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

    // The hard-blocked child is NOT started and shows as blocked-by db.
    assert_eq!(
        proj.blocked_by("feature-api"),
        vec!["feature-db".to_string()]
    );
    let api = proj.tasks.iter().find(|t| t.id == "feature-api").unwrap();
    assert_eq!(api.status, casting::projection::TaskStatus::Backlog);

    // The READY children are started (kicked in parallel).
    for ready in ["feature-db", "feature-ui", "feature-sec"] {
        let t = proj.tasks.iter().find(|t| t.id == ready).unwrap();
        assert_eq!(
            t.status,
            casting::projection::TaskStatus::Working,
            "{ready} should be kicked in parallel"
        );
    }

    // The graph surfaces the group + the ordering.
    let g = proj.graph();
    assert_eq!(g.groups.len(), 1, "feature is a join point");
    let api_node = g.nodes.iter().find(|n| n.task_id == "feature-api").unwrap();
    assert_eq!(api_node.blocked_by, vec!["feature-db".to_string()]);
    assert!(
        !api_node.transitions.contains(&"start".to_string()),
        "blocked child must not expose `start`"
    );
}

#[tokio::test]
async fn default_onboard_without_decompose_stays_flat() {
    // Regression guard: with decompose off, the canonical flow is unchanged —
    // no feature children are fanned out.
    let store = casting::sqlite_store::SqliteEventStore::in_memory().unwrap();
    let cursors = casting::cursor::SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj").with_step_delay(Duration::ZERO);
    seed(&state);

    state
        .append(casting::event::Event::new(
            "proj",
            casting::event::Actor::Owner,
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
    assert!(
        proj.tasks.iter().all(|t| t.parent_id.is_none()),
        "no decomposition when the flag is off"
    );
    assert!(proj.children_of("task-feature").is_empty());
}
