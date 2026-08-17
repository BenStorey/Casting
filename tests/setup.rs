//! Tests for the setup engine (`cast init`) — onboarding as a shared,
//! deterministic flow (director decision 2026-08-10: CLI + UI share one engine).
//!
//! The cast roster is `active-cast/` (the consultant registry). Setup seeds the
//! project + config; hiring happens via the roster reconcile (`cast_roster`)
//! or `ensure_hires` — never hardcoded names/counters.

use casting::projection::Projection;
use casting::runtime::directive::{DirectiveKind, DirectiveStrength};
use casting::store::EventStore;
use casting::workspace::setup::{self, SetupPlan, SetupSpec, StartDirective};

fn tmp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// Boot an AppState over a setup state dir and reconcile the cast roster from
/// the directory (what `cast run` does at boot). Returns the state (hired).
fn reconcile(dir: &std::path::Path) -> casting::pm::AppState {
    use casting::pm::reconciler::cast_roster;
    use casting::pm::AppState;
    use casting::store::SqliteCursorStore;
    let store = setup::open_store(dir).unwrap();
    let cursors = SqliteCursorStore::open(dir.join("cursors.db")).unwrap();
    let state = AppState::new(store, cursors, "project-demo");
    cast_roster(&state).unwrap();
    state
}

fn default_spec() -> SetupSpec {
    SetupSpec {
        name: "Acme Inc".into(),
        roles: vec![], // empty = whole roster
        director_token: None,
        directives: vec![],
    }
}

#[test]
fn setup_seeds_project_and_config_without_hardcoded_hires() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(default_spec()).unwrap();
    let written = plan.apply(tmp.path()).unwrap();
    // ProjectCreated + BudgetSet only — NO hardcoded hire events.
    assert_eq!(written, 2, "setup writes project + budget, not hires");

    let store = setup::open_store(tmp.path()).unwrap();
    let proj = Projection::build(&store, "project-demo").unwrap();
    assert!(
        proj.agents.is_empty(),
        "apply_to_store must NOT write hires (the roster reconcile does)"
    );
    // Company name recorded on the project.
    assert!(store.latest_sequence("project-demo").unwrap() >= 2);
    // Config persisted.
    let cfg = setup::read_config(tmp.path()).expect("config written");
    assert_eq!(cfg.name, "Acme Inc");
}

#[test]
fn setup_then_reconcile_hires_full_roster() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(default_spec()).unwrap();
    plan.apply(tmp.path()).unwrap();

    // Reconcile the roster from the directory: PM, Advisor, all assignable.
    let state = reconcile(tmp.path());
    let proj = Projection::build(&state.store, "project-demo").unwrap();
    for id in &["mei", "jeeves", "diego", "tess", "nina", "ali", "julien"] {
        assert!(
            proj.agents.iter().any(|a| a.id == *id),
            "roster must include {id}"
        );
    }
    assert_eq!(proj.agents.len(), 7, "one consultant per role");
}

#[test]
fn ensure_hires_selects_roles_or_whole_roster() {
    use casting::pm::AppState;
    use casting::store::SqliteCursorStore;

    let store = casting::store::SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "project-demo");
    state
        .append(casting::event::Event::new(
            "project-demo",
            casting::event::Actor::System,
            casting::event::EventType::ProjectCreated,
            casting::event::Aggregate {
                kind: "project".into(),
                id: "project-demo".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();

    // Specific roles → exactly those consultants (one per role).
    let hires = setup::ensure_hires(
        &state,
        &["testing-engineer".into(), "systems-architect".into()],
    )
    .unwrap();
    assert_eq!(hires.len(), 2);
    let mut proj = Projection::build(&state.store, "project-demo").unwrap();
    let ids: Vec<&str> = proj.agents.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, vec!["tess", "nina"], "only the requested roles hired");

    // Empty roles → the whole roster (the directory IS the roster).
    let hires_all = setup::ensure_hires(&state, &[]).unwrap();
    assert_eq!(hires_all.len(), 5, "the remaining 5 consultants added");
    proj = Projection::build(&state.store, "project-demo").unwrap();
    assert_eq!(proj.agents.len(), 7, "full roster: one per role");
}

#[test]
fn setup_is_idempotent_on_rerun() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(default_spec()).unwrap();
    let first = plan.apply(tmp.path()).unwrap();
    assert!(first > 0, "first run writes events");

    let second = plan.apply(tmp.path()).unwrap();
    assert_eq!(second, 0, "re-run is a no-op");

    let store = setup::open_store(tmp.path()).unwrap();
    let proj = Projection::build(&store, "project-demo").unwrap();
    assert!(
        proj.agents.is_empty(),
        "no duplicate hires — apply never writes hire events"
    );
}

#[test]
fn setup_writes_director_token_config() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(SetupSpec {
        name: "Acme".into(),
        roles: vec![],
        director_token: Some("s3cr3t".into()),
        directives: vec![],
    })
    .unwrap();
    plan.apply(tmp.path()).unwrap();
    let cfg = setup::read_config(tmp.path()).expect("config written");
    assert_eq!(cfg.name, "Acme");
    assert_eq!(cfg.director_token.as_deref(), Some("s3cr3t"));

    // A re-run (no-op) must NOT clobber the persisted token/config.
    let again = SetupPlan::build(default_spec()).unwrap();
    assert_eq!(again.apply(tmp.path()).unwrap(), 0, "re-run is a no-op");
    let cfg = setup::read_config(tmp.path()).expect("config still present");
    assert_eq!(
        cfg.director_token.as_deref(),
        Some("s3cr3t"),
        "re-run must not wipe the token"
    );
}

#[test]
fn setup_writes_starting_directives() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(SetupSpec {
        name: "Acme".into(),
        roles: vec![],
        director_token: None,
        directives: vec![StartDirective {
            id: "d-tdd".into(),
            kind: DirectiveKind::Policy,
            statement: "TDD required".into(),
            scope: vec!["engineering".into()],
            strength: DirectiveStrength::Required,
        }],
    })
    .unwrap();
    plan.apply(tmp.path()).unwrap();

    let store = setup::open_store(tmp.path()).unwrap();
    let proj = Projection::build(&store, "project-demo").unwrap();
    assert!(proj.directives.iter().any(|d| d.id == "d-tdd"));
}

#[test]
fn setup_rejects_unknown_role() {
    let spec = SetupSpec {
        name: "Acme".into(),
        roles: vec!["wizard".into()],
        director_token: None,
        directives: vec![],
    };
    assert!(
        SetupPlan::build(spec).is_err(),
        "unknown role must be rejected"
    );
}

#[tokio::test]
async fn setup_then_onboard_does_not_double_hire() {
    // Setup seeds the project; the roster reconcile hires the cast. Then the
    // director's first message drives onboarding — plan_onboard must skip
    // already-hired agents so we don't get duplicates.
    let tmp = tmp_dir();
    let plan = SetupPlan::build(default_spec()).unwrap();
    plan.apply(tmp.path()).unwrap();
    let state = reconcile(tmp.path());

    use casting::event::{Actor, Aggregate, Event, EventType};
    use casting::runtime::orchestrator::MockOrchestrator;
    use std::sync::Arc;
    let state = state.with_orchestrator(Arc::new(MockOrchestrator));
    state
        .append(Event::new(
            "project-demo",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-1".into(),
            },
            serde_json::json!({ "body": "Build me a todo app" }),
        ))
        .unwrap();
    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "project-demo").unwrap();
    // Exactly one of each roster agent — no duplicates from onboarding.
    for expected in ["mei", "jeeves", "diego", "tess", "nina", "ali", "julien"] {
        let count = proj.agents.iter().filter(|a| a.id == expected).count();
        assert_eq!(count, 1, "agent {expected} hired exactly once, got {count}");
    }
}
