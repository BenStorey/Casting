//! Tests for the Project Plan + priority reducer (mature-the-state-core item 2).
//!
//! Per docs/SEMANTIC_EVENTS.md: events are mutations, projections are state.
//! A `TaskPriorityChanged` event is a fact; `task.priority` and the derived
//! `ProjectPlan` are deterministic state. This is the first dogfooding artifact
//! (our own roadmap would become this derived state, not `.md`).

use casting::cursor::SqliteCursorStore;
use casting::event::{Actor, Event, EventType};
use casting::plan::{PlannedItem, Priority, ProjectPlan};
use casting::pm::AppState;
use casting::projection::{Projection, TaskStatus};
use casting::sqlite_store::SqliteEventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-plan")
}

fn create_task(state: &AppState, id: &str, title: &str) {
    state
        .append(Event::new(
            "proj-plan",
            Actor::Agent { id: "pm".into() },
            EventType::TaskCreated,
            casting::event::Aggregate {
                kind: "task".into(),
                id: id.into(),
            },
            serde_json::json!({ "title": title, "kind": "feature" }),
        ))
        .unwrap();
}

fn set_priority(state: &AppState, task: &str, from: &str, to: &str) {
    state
        .append(Event::new(
            "proj-plan",
            Actor::Agent { id: "pm".into() },
            EventType::TaskPriorityChanged,
            casting::event::Aggregate {
                kind: "task".into(),
                id: task.into(),
            },
            serde_json::json!({ "task_id": task, "from": from, "to": to }),
        ))
        .unwrap();
}

// --- Task 1: Priority ---

#[test]
fn priority_ordering_is_critical_high_medium_low() {
    assert!(Priority::Critical > Priority::High);
    assert!(Priority::High > Priority::Medium);
    assert!(Priority::Medium > Priority::Low);
    assert!(Priority::Low < Priority::Critical);
}

#[test]
fn priority_defaults_to_medium() {
    assert_eq!(Priority::default(), Priority::Medium);
}

#[test]
fn priority_round_trips_through_json() {
    for p in [
        Priority::Critical,
        Priority::High,
        Priority::Medium,
        Priority::Low,
    ] {
        let back: Priority = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }
}

// --- Task 2: reducer ---

#[test]
fn creating_a_task_defaults_priority_to_medium() {
    let state = make_state();
    create_task(&state, "task-a", "Auth");
    let proj = Projection::build(&state.store, "proj-plan").unwrap();
    assert_eq!(proj.tasks[0].priority, Priority::Medium);
}

#[test]
fn priority_change_reduces_into_task_priority() {
    let state = make_state();
    create_task(&state, "task-auth", "Authentication");
    set_priority(&state, "task-auth", "high", "low");
    let proj = Projection::build(&state.store, "proj-plan").unwrap();
    let auth = proj.tasks.iter().find(|t| t.id == "task-auth").unwrap();
    assert_eq!(auth.priority, Priority::Low);
}

// --- Task 4: plan derivation ---

#[test]
fn plan_orders_tasks_by_priority_and_lists_open_decisions() {
    let state = make_state();

    // Objective: an open requirement.
    state
        .append(Event::new(
            "proj-plan",
            Actor::Agent { id: "pm".into() },
            EventType::RequirementCreated,
            casting::event::Aggregate {
                kind: "requirement".into(),
                id: "req-1".into(),
            },
            serde_json::json!({ "title": "Build a climbing gym SaaS", "description": "..." }),
        ))
        .unwrap();

    create_task(&state, "task-auth", "Authentication");
    create_task(&state, "task-qa", "QA setup");
    create_task(&state, "task-billing", "Billing");
    create_task(&state, "task-analytics", "Analytics");
    set_priority(&state, "task-billing", "medium", "high");
    set_priority(&state, "task-auth", "high", "low");

    // An open decision waiting on the owner.
    state
        .append(Event::new(
            "proj-plan",
            Actor::Agent { id: "pm".into() },
            EventType::DecisionProposed,
            casting::event::Aggregate {
                kind: "decision".into(),
                id: "decision-db".into(),
            },
            serde_json::json!({
                "subject": "Database choice",
                "options": serde_json::json!({}),
                "recommendation": "A",
                "class": "database",
                "involvement": "ask",
            }),
        ))
        .unwrap();

    let proj = Projection::build(&state.store, "proj-plan").unwrap();
    let plan = proj.plan();

    assert_eq!(plan.objective.as_deref(), Some("Build a climbing gym SaaS"));

    // Ordered Critical..Low: billing (high) before the medium tasks, auth low last.
    let order: Vec<&str> = plan.priorities.iter().map(|i| i.task_id.as_str()).collect();
    let billing = order.iter().position(|x| *x == "task-billing").unwrap();
    let auth = order.iter().position(|x| *x == "task-auth").unwrap();
    let qa = order.iter().position(|x| *x == "task-qa").unwrap();
    assert!(
        billing < qa,
        "high-priority billing should rank before medium QA"
    );
    assert!(qa < auth, "medium QA should rank before low auth");
    // Every task appears exactly once.
    assert_eq!(order.len(), 4);

    // Auth is deprioritized (the only Low task).
    assert_eq!(
        plan.deprioritized
            .iter()
            .map(|i| i.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-auth"]
    );

    // Decision is open (in the owner's inbox).
    assert_eq!(plan.open_decisions, vec!["Database choice".to_string()]);
}

#[test]
fn plan_item_and_plan_are_serializable() {
    let item = PlannedItem {
        task_id: "task-a".into(),
        title: "Auth".into(),
        priority: Priority::High,
    };
    let plan = ProjectPlan {
        objective: Some("objective".into()),
        priorities: vec![item],
        deprioritized: vec![],
        open_risks: vec![],
        active_directives: vec![],
        open_decisions: vec![],
    };
    let json = serde_json::to_string(&plan).unwrap();
    assert!(json.contains("task-a"));
    assert!(json.contains("high"));
}

#[test]
fn done_tasks_are_excluded_from_current_priorities() {
    let state = make_state();
    create_task(&state, "task-done", "Finished thing");
    state
        .append(Event::new(
            "proj-plan",
            Actor::Agent { id: "pm".into() },
            EventType::TaskCompleted,
            casting::event::Aggregate {
                kind: "task".into(),
                id: "task-done".into(),
            },
            serde_json::json!({ "result": "done" }),
        ))
        .unwrap();
    let proj = Projection::build(&state.store, "proj-plan").unwrap();
    assert_eq!(proj.tasks[0].status, TaskStatus::Done);
    let plan = proj.plan();
    assert!(
        plan.priorities.is_empty(),
        "a done task should not appear in current priorities"
    );
}

// --- Task 3: SetTaskPriority through the gate ---

#[test]
fn set_priority_validates_against_existing_task() {
    use casting::actions::{validate, PolicyError};
    let state = make_state();
    create_task(&state, "task-a", "Auth");
    let proj = Projection::build(&state.store, "proj-plan").unwrap();

    // Setting priority on an existing task passes the gate.
    let action = casting::actions::PmAction::SetTaskPriority {
        task_id: "task-a".into(),
        priority: Priority::High,
    };
    assert!(validate(&action, "pm", &proj).is_ok());

    // On a missing task it's rejected.
    let err = validate(
        &casting::actions::PmAction::SetTaskPriority {
            task_id: "task-missing".into(),
            priority: Priority::Critical,
        },
        "pm",
        &proj,
    )
    .expect_err("setting priority on a missing task must be rejected");
    assert!(matches!(err, PolicyError::TaskNotFound(_)));
}

#[test]
fn set_priority_action_emits_task_priority_changed_and_reduces() {
    let state = make_state();
    create_task(&state, "task-a", "Auth");
    // Verify the action -> event mapping + reducer together.
    let cause = Event::new(
        "proj-plan",
        Actor::Agent { id: "pm".into() },
        EventType::MessageSent,
        casting::event::Aggregate {
            kind: "message".into(),
            id: "msg-1".into(),
        },
        serde_json::json!({ "to": "pm", "body": "prioritize auth" }),
    );
    let evs = casting::actions::PmAction::SetTaskPriority {
        task_id: "task-a".into(),
        priority: Priority::Critical,
    }
    .to_events("proj-plan", "pm", &cause, "corr-1");
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].event_type, EventType::TaskPriorityChanged);
    assert_eq!(evs[0].data["to"], serde_json::json!("critical"));

    // Append it and confirm the projection reduces to Critical.
    for e in &evs {
        state.append(e.clone()).unwrap();
    }
    let proj = Projection::build(&state.store, "proj-plan").unwrap();
    let auth = proj.tasks.iter().find(|t| t.id == "task-a").unwrap();
    assert_eq!(auth.priority, Priority::Critical);
}
