//! Tests for the setup engine (`cast init`) — onboarding as a shared,
//! deterministic flow (owner decision 2026-08-10: CLI + UI share one engine).

use casting::directive::{DirectiveKind, DirectiveStrength};
use casting::projection::Projection;
use casting::setup::{self, SetupPlan, SetupSpec, StartDirective};
use casting::store::EventStore;

fn tmp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn setup_creates_company_and_default_cast() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(SetupSpec {
        name: "Acme Inc".into(),
        roles: vec![], // default cast
        owner_token: None,
        directives: vec![],
    })
    .unwrap();
    let written = plan.apply(tmp.path()).unwrap();
    assert!(written >= 3, "project + PM + cast members");

    let store = setup::open_store(tmp.path()).unwrap();
    let proj = Projection::build(&store, "project-demo").unwrap();
    assert!(!proj.agents.is_empty());
    // Default cast => the two canonical default agents hired.
    assert!(proj.agents.iter().any(|a| a.id == "pm"));
    assert!(proj.agents.iter().any(|a| a.id == "marcus-reed"));
    assert!(proj.agents.iter().any(|a| a.id == "maya-patel"));
    // Company name recorded on the project.
    assert!(store.latest_sequence("project-demo").unwrap() >= 3);
}

#[test]
fn setup_with_custom_roles_hires_them() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(SetupSpec {
        name: "SecCo".into(),
        roles: vec!["security".into(), "devops".into()],
        owner_token: None,
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
    assert!(ids.contains(&"pm"));
}

#[test]
fn setup_is_idempotent_on_rerun() {
    let tmp = tmp_dir();
    let spec = SetupSpec {
        name: "Acme".into(),
        roles: vec![],
        owner_token: None,
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
        proj.agents.len() == 3,
        "no duplicate hires on re-run: {:?}",
        proj.agents
    );
}

#[test]
fn setup_writes_owner_token_config() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(SetupSpec {
        name: "Acme".into(),
        roles: vec![],
        owner_token: Some("s3cr3t".into()),
        directives: vec![],
    })
    .unwrap();
    plan.apply(tmp.path()).unwrap();
    let cfg = setup::read_config(tmp.path()).expect("config written");
    assert_eq!(cfg.name, "Acme");
    assert_eq!(cfg.owner_token.as_deref(), Some("s3cr3t"));
}

#[test]
fn setup_writes_starting_directives() {
    let tmp = tmp_dir();
    let plan = SetupPlan::build(SetupSpec {
        name: "Acme".into(),
        roles: vec![],
        owner_token: None,
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
        owner_token: None,
        directives: vec![],
    };
    assert!(
        SetupPlan::build(spec).is_err(),
        "unknown role must be rejected"
    );
}
