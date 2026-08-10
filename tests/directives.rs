//! Tests for Project Directives (docs/INTENT.md governance layer).
//!
//! Directives are first-class, event-sourced governance state: policies,
//! constraints, principles, practices, preferences, objectives. Task 1 covers
//! the model + context resolver; later tasks cover reducers and the gate.

use casting::directive::{self, Directive, DirectiveKind, DirectiveStatus, DirectiveStrength};
use casting::projection::Projection;

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
