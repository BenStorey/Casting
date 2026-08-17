//! Tests for the setup engine (`cast init`) — onboarding as a shared,
//! deterministic flow (director decision 2026-08-10: CLI + UI share one engine).

use casting::projection::Projection;
use casting::runtime::directive::{DirectiveKind, DirectiveStrength};
use casting::store::EventStore;
use casting::workspace::setup::{self, SetupPlan, SetupSpec, StartDirective};

fn tmp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn setup_creates_company_and_default_cast() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(SetupSpec {
        name: "Acme Inc".into(),
        roles: vec![], // default cast
        director_token: None,
        directives: vec![],
    })
    .unwrap();
    let written = plan.apply(tmp.path()).unwrap();
    assert!(written >= 7, "project + PM + 5 assignable cast members");

    let store = setup::open_store(tmp.path()).unwrap();
    let proj = Projection::build(&store, "project-demo").unwrap();
    assert!(!proj.agents.is_empty());
    // Default cast => PM + 5 assignable consultants (2026-08-14 roster).
    for id in &["mei", "diego", "tess", "nina", "ali", "julien"] {
        assert!(
            proj.agents.iter().any(|a| a.id == *id),
            "default cast must include {id}"
        );
    }
    assert_eq!(proj.agents.len(), 6, "pm + 5 assignable");
    // Company name recorded on the project.
    assert!(store.latest_sequence("project-demo").unwrap() >= 7);
}

#[test]
fn setup_with_custom_roles_hires_them() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(SetupSpec {
        name: "SecCo".into(),
        roles: vec!["security".into(), "devops".into()],
        director_token: None,
        directives: vec![],
    })
    .unwrap();
    plan.apply(tmp.path()).unwrap();

    let store = setup::open_store(tmp.path()).unwrap();
    let proj = Projection::build(&store, "project-demo").unwrap();
    // The custom cast replaces the default — security + devops hired, but NOT
    // the default engineer/qa agents.
    let ids: Vec<&str> = proj.agents.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"security-1"));
    assert!(ids.contains(&"devops-1"));
    assert!(!ids.contains(&"marcus-reed"));
    assert!(ids.contains(&"mei"));
}

#[test]
fn setup_is_idempotent_on_rerun() {
    let tmp = tmp_dir();
    let spec = SetupSpec {
        name: "Acme".into(),
        roles: vec![],
        director_token: None,
        directives: vec![],
    };
    let plan = SetupPlan::build(spec).unwrap();
    let first = plan.apply(tmp.path()).unwrap();
    assert!(first > 0, "first run writes events");

    let second = plan.apply(tmp.path()).unwrap();
    assert_eq!(second, 0, "re-run is a no-op");

    let store = setup::open_store(tmp.path()).unwrap();
    let proj = Projection::build(&store, "project-demo").unwrap();
    assert!(
        proj.agents.len() == 6,
        "no duplicate hires on re-run: {:?}",
        proj.agents
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
    let again = SetupPlan::build(SetupSpec {
        name: "Acme".into(),
        roles: vec![],
        director_token: None,
        directives: vec![],
    })
    .unwrap();
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
    // Setup seeds the cast; then the director's first message drives onboarding.
    // plan_onboard must skip already-hired agents so we don't get duplicates.
    let tmp = tmp_dir();
    let plan = SetupPlan::build(SetupSpec {
        name: "Acme".into(),
        roles: vec![], // default cast (marcus + maya) hired by setup
        director_token: None,
        directives: vec![],
    })
    .unwrap();
    plan.apply(tmp.path()).unwrap();

    // Boot an AppState over the setup state dir and drive the PM with the
    // director's first message (which triggers plan_onboard).
    use casting::event::{Actor, Aggregate, Event, EventType};
    use casting::pm::AppState;
    use casting::runtime::orchestrator::MockOrchestrator;
    use casting::store::SqliteCursorStore;
    use std::sync::Arc;
    let store = casting::workspace::setup::open_store(tmp.path()).unwrap();
    let cursors = SqliteCursorStore::open(tmp.path().join("cursors.db")).unwrap();
    let state =
        AppState::new(store, cursors, "project-demo").with_orchestrator(Arc::new(MockOrchestrator));

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
    // Exactly one of each default agent — no duplicates from onboarding.
    for expected in ["mei", "diego", "tess", "nina", "ali", "julien"] {
        let count = proj.agents.iter().filter(|a| a.id == expected).count();
        assert_eq!(count, 1, "agent {expected} hired exactly once, got {count}");
    }
}

#[tokio::test]
async fn setup_custom_cast_is_not_topped_up_by_onboarding() {
    // the director chose a custom cast at setup (engineer + devops only). Onboarding
    // must NOT add default QA on top — the chosen team stands as-is.
    let tmp = tmp_dir();
    let plan = SetupPlan::build(SetupSpec {
        name: "Acme".into(),
        roles: vec!["engineer".into(), "devops".into()],
        director_token: None,
        directives: vec![],
    })
    .unwrap();
    plan.apply(tmp.path()).unwrap();

    use casting::event::{Actor, Aggregate, Event, EventType};
    use casting::pm::AppState;
    use casting::runtime::orchestrator::MockOrchestrator;
    use casting::store::SqliteCursorStore;
    use std::sync::Arc;
    let store = casting::workspace::setup::open_store(tmp.path()).unwrap();
    let cursors = SqliteCursorStore::open(tmp.path().join("cursors.db")).unwrap();
    let state =
        AppState::new(store, cursors, "project-demo").with_orchestrator(Arc::new(MockOrchestrator));
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
            serde_json::json!({ "body": "Build me a thing" }),
        ))
        .unwrap();
    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "project-demo").unwrap();
    let ids: Vec<&str> = proj.agents.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"engineer-1"), "engineer in the custom cast");
    assert!(ids.contains(&"devops-1"), "devops in the custom cast");
    assert!(
        !ids.contains(&"diego"),
        "default Lead Developer must NOT be topped up: {ids:?}"
    );
    assert!(!ids.contains(&"critic"), "no other defaults added: {ids:?}");
}
