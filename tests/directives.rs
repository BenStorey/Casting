//! Tests for Project Directives (docs/INTENT.md governance layer).
//!
//! Directives are first-class, event-sourced governance state: policies,
//! constraints, principles, practices, preferences, objectives. Task 1 covers
//! the model + context resolver; later tasks cover reducers and the gate.

use casting::cursor::CursorStore;
use casting::directive::{self, Directive, DirectiveKind, DirectiveStatus, DirectiveStrength};
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::AppState;
use casting::projection::Projection;
use casting::sqlite_store::SqliteEventStore;

fn make_state() -> AppState {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = CursorStore::in_memory().unwrap();
    AppState::new(store, cursors, "proj-dir")
}

fn append(state: &AppState, event_type: EventType, id: &str, data: serde_json::Value) {
    state
        .append(Event::new(
            "proj-dir",
            Actor::Owner,
            event_type,
            Aggregate {
                kind: "directive".into(),
                id: id.into(),
            },
            data,
        ))
        .unwrap();
}

// --- Task 2: reducer lifecycle ---

#[test]
fn directive_created_reduces_to_active() {
    let state = make_state();
    append(
        &state,
        EventType::ProjectDirectiveCreated,
        "d-tdd",
        serde_json::json!({
            "kind": "policy",
            "statement": "Use TDD",
            "scope": ["engineering"],
            "strength": "required",
            "created_by": "owner",
            "supersedes": null,
        }),
    );
    let proj = Projection::build(&state.store, "proj-dir").unwrap();
    assert_eq!(proj.directives.len(), 1);
    let d = &proj.directives[0];
    assert_eq!(d.id, "d-tdd");
    assert_eq!(d.kind, DirectiveKind::Policy);
    assert_eq!(d.strength, DirectiveStrength::Required);
    assert_eq!(d.status, DirectiveStatus::Active);
    assert_eq!(d.created_by, "owner");
    assert_eq!(d.scope, vec!["engineering".to_string()]);
}

#[test]
fn directive_lifecycle_transitions_status() {
    let state = make_state();
    append(
        &state,
        EventType::ProjectDirectiveCreated,
        "d-2",
        serde_json::json!({
            "kind": "constraint",
            "statement": "Budget under 250",
            "scope": ["finance"],
            "strength": "strong",
            "created_by": "owner",
        }),
    );
    append(
        &state,
        EventType::ProjectDirectiveSuspended,
        "d-2",
        serde_json::json!({}),
    );
    let proj = Projection::build(&state.store, "proj-dir").unwrap();
    assert_eq!(proj.directives[0].status, DirectiveStatus::Suspended);

    append(
        &state,
        EventType::ProjectDirectiveResumed,
        "d-2",
        serde_json::json!({}),
    );
    let proj = Projection::build(&state.store, "proj-dir").unwrap();
    assert_eq!(proj.directives[0].status, DirectiveStatus::Active);

    append(
        &state,
        EventType::ProjectDirectiveExpired,
        "d-2",
        serde_json::json!({}),
    );
    let proj = Projection::build(&state.store, "proj-dir").unwrap();
    assert_eq!(proj.directives[0].status, DirectiveStatus::Expired);
}

#[test]
fn directive_superseded_preserves_history_and_marks_superseded() {
    let state = make_state();
    append(
        &state,
        EventType::ProjectDirectiveCreated,
        "d-v1",
        serde_json::json!({
            "kind": "policy",
            "statement": "SQLite everywhere",
            "scope": ["architecture"],
            "strength": "strong",
            "created_by": "owner",
        }),
    );
    append(
        &state,
        EventType::ProjectDirectiveCreated,
        "d-v2",
        serde_json::json!({
            "kind": "policy",
            "statement": "Postgres for prod",
            "scope": ["architecture"],
            "strength": "required",
            "created_by": "owner",
            "supersedes": "d-v1",
        }),
    );
    append(
        &state,
        EventType::ProjectDirectiveSuperseded,
        "d-v1",
        serde_json::json!({}),
    );
    let proj = Projection::build(&state.store, "proj-dir").unwrap();

    // Both are persisted (history preserved).
    assert_eq!(proj.directives.len(), 2);
    let v1 = proj.directives.iter().find(|d| d.id == "d-v1").unwrap();
    assert_eq!(v1.status, DirectiveStatus::Superseded);
    let v2 = proj.directives.iter().find(|d| d.id == "d-v2").unwrap();
    assert_eq!(v2.status, DirectiveStatus::Active);
    assert_eq!(v2.supersedes.as_deref(), Some("d-v1"));
}

// --- Task 1: model ---
#[test]
fn directive_new_defaults_to_active() {
    let d = Directive::new(
        "d-1".into(),
        DirectiveKind::Policy,
        "Use TDD".into(),
        vec!["engineering".into()],
        DirectiveStrength::Required,
        "owner".into(),
        None,
    );
    assert_eq!(d.status, DirectiveStatus::Active);
    assert_eq!(d.strength, DirectiveStrength::Required);
    assert_eq!(d.scope, vec!["engineering".to_string()]);
}

#[test]
fn strength_ordering_is_required_strong_recommended() {
    assert!(DirectiveStrength::Required > DirectiveStrength::Strong);
    assert!(DirectiveStrength::Strong > DirectiveStrength::Recommended);
    assert!(DirectiveStrength::Recommended < DirectiveStrength::Required);
}

#[test]
fn directive_types_round_trip_through_json() {
    let d = Directive::new(
        "d-2".into(),
        DirectiveKind::Constraint,
        "Budget under 250".into(),
        vec!["finance".into()],
        DirectiveStrength::Strong,
        "pm".into(),
        None,
    );
    let json = serde_json::to_string(&d).unwrap();
    let back: Directive = serde_json::from_str(&json).unwrap();
    assert_eq!(back.kind, DirectiveKind::Constraint);
    assert_eq!(back.strength, DirectiveStrength::Strong);
}

#[test]
fn relevant_filters_by_scope_and_status_and_orders_by_strength() {
    // Seed directives directly into a projection (Task 2 adds real reducers).
    let mut proj = Projection {
        project_id: "proj-dir".into(),
        ..Default::default()
    };
    proj.directives.push(Directive::new(
        "d-tdd".into(),
        DirectiveKind::Policy,
        "TDD required".into(),
        vec!["engineering".into()],
        DirectiveStrength::Required,
        "owner".into(),
        None,
    ));
    proj.directives.push(Directive::new(
        "d-simplicity".into(),
        DirectiveKind::Principle,
        "Prefer simple".into(),
        vec!["architecture".into(), "engineering".into()],
        DirectiveStrength::Strong,
        "owner".into(),
        None,
    ));
    proj.directives.push(Directive::new(
        "d-suspended".into(),
        DirectiveKind::Preference,
        "Postgres".into(),
        vec!["engineering".into()],
        DirectiveStrength::Recommended,
        "owner".into(),
        None,
    ));
    // Suspend the last one to show status filtering.
    proj.directives[2].status = DirectiveStatus::Suspended;

    let eng = directive::relevant(&proj, &["engineering"]);
    let ids: Vec<&str> = eng.iter().map(|d| d.id.as_str()).collect();
    // Only active + in-scope; strongest first (Required before Strong).
    assert_eq!(ids, vec!["d-tdd", "d-simplicity"]);

    // Architecture-only context: only the simplicity principle.
    let arch = directive::relevant(&proj, &["architecture"]);
    let ids: Vec<&str> = arch.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, vec!["d-simplicity"]);

    // A context with no overlap: nothing.
    let none = directive::relevant(&proj, &["finance"]);
    assert!(none.is_empty());
}

// --- Task 3: authority gate ---

fn build_projection_with_directives() -> Projection {
    let mut proj = Projection::default();
    proj.directives.push(Directive::new(
        "d-tdd".into(),
        DirectiveKind::Policy,
        "TDD required".into(),
        vec!["engineering".into()],
        DirectiveStrength::Required,
        "owner".into(),
        None,
    ));
    proj.directives.push(Directive::new(
        "d-v2".into(),
        DirectiveKind::Policy,
        "Postgres".into(),
        vec!["architecture".into()],
        DirectiveStrength::Required,
        "owner".into(),
        None,
    ));
    proj
}

#[test]
fn owner_can_create_and_suspend_directives() {
    use casting::actions::{validate, PmAction};
    let proj = Projection::default();
    let create = PmAction::CreateDirective {
        id: "d-1".into(),
        kind: DirectiveKind::Constraint,
        statement: "Budget under 250".into(),
        scope: vec!["finance".into()],
        strength: DirectiveStrength::Strong,
        supersedes: None,
    };
    assert!(validate(&create, "owner", &proj).is_ok());
}

#[test]
fn plain_agent_cannot_change_governance() {
    use casting::actions::{validate, PmAction, PolicyError};
    let proj = build_projection_with_directives();

    let create = PmAction::CreateDirective {
        id: "d-agent".into(),
        kind: DirectiveKind::Preference,
        statement: "I'd like X".into(),
        scope: vec!["engineering".into()],
        strength: DirectiveStrength::Recommended,
        supersedes: None,
    };
    let err = validate(&create, "marcus-reed", &proj)
        .expect_err("a plain engineer agent must not change governance");
    assert!(matches!(err, PolicyError::DirectiveAuthority(_)));

    let suspend = PmAction::SuspendDirective {
        directive_id: "d-tdd".into(),
    };
    let err = validate(&suspend, "maya-patel", &proj)
        .expect_err("a plain QA agent must not change governance");
    assert!(matches!(err, PolicyError::DirectiveAuthority(_)));
}

#[test]
fn pm_and_system_may_change_governance() {
    use casting::actions::{validate, PmAction};
    let proj = build_projection_with_directives();

    for who in ["pm", "system"] {
        let create = PmAction::CreateDirective {
            id: format!("d-{who}"),
            kind: DirectiveKind::Policy,
            statement: "Some rule".into(),
            scope: vec!["engineering".into()],
            strength: DirectiveStrength::Strong,
            supersedes: None,
        };
        assert!(validate(&create, who, &proj).is_ok(), "create by {who}");
        let suspend = PmAction::SuspendDirective {
            directive_id: "d-tdd".into(),
        };
        assert!(validate(&suspend, who, &proj).is_ok(), "suspend by {who}");
    }
}

#[test]
fn suspending_a_missing_directive_is_rejected() {
    use casting::actions::{validate, PmAction, PolicyError};
    let proj = build_projection_with_directives();
    let err = validate(
        &PmAction::SuspendDirective {
            directive_id: "d-nope".into(),
        },
        "owner",
        &proj,
    )
    .expect_err("suspending a missing directive must be rejected");
    assert!(matches!(err, PolicyError::DirectiveNotFound(_)));
}

#[test]
fn supersede_requires_an_existing_active_target() {
    use casting::actions::{validate, PmAction, PolicyError};
    let proj = build_projection_with_directives();

    // Valid: d-tdd (exists, active) superseded by d-v2 (exists, active) — but
    // for the Supersede action `by` is the *target*, d-v2. This is allowed.
    let ok = validate(
        &PmAction::SupersedeDirective {
            directive_id: "d-tdd".into(),
            by_directive_id: "d-v2".into(),
        },
        "owner",
        &proj,
    );
    assert!(ok.is_ok());

    // Invalid: `by` doesn't exist.
    let err = validate(
        &PmAction::SupersedeDirective {
            directive_id: "d-tdd".into(),
            by_directive_id: "d-missing".into(),
        },
        "owner",
        &proj,
    )
    .expect_err("superseding onto a missing directive must be rejected");
    assert!(matches!(err, PolicyError::DirectiveNotFound(_)));
}
