//! Tests for the consultant registry + loader: curated embedded defaults,
//! default-cast resolution, strict validation, and user-directory overlays.

use casting::consultants::ConsultantRegistry;

#[test]
fn embedded_defaults_cover_all_catalog_roles() {
    let reg = ConsultantRegistry::from_embedded().expect("defaults parse + validate");
    // One package per catalog role.
    assert_eq!(reg.count(), 4);

    for role in ["engineer", "qa", "security", "devops"] {
        let c = reg
            .for_role(role)
            .unwrap_or_else(|| panic!("no consultant bound to role {role}"));
        assert_eq!(c.role, role);
        assert!(!c.name.is_empty(), "role {role} consultant has a name");
        assert!(
            !c.scope.is_empty(),
            "role {role} consultant carries a scope"
        );
    }
}

#[test]
fn default_cast_is_the_two_core_members() {
    let reg = ConsultantRegistry::from_embedded().unwrap();
    let default: Vec<String> = reg
        .default_cast()
        .into_iter()
        .map(|c| c.id.to_string())
        .collect();

    // Engineers + QA are always on; specialists are summoned, not default.
    assert!(default.contains(&"marcus-reed".to_string()));
    assert!(default.contains(&"maya-patel".to_string()));
    assert!(!default.contains(&"devon-carter".to_string()));
    assert!(!default.contains(&"priya-sharma".to_string()));
}

#[test]
fn every_embedded_system_prompt_is_loaded() {
    let reg = ConsultantRegistry::from_embedded().unwrap();
    for c in reg.all() {
        assert!(
            c.system_prompt.is_some(),
            "consultant {} must ship a system prompt",
            c.id
        );
        assert!(
            !c.system_prompt.as_deref().unwrap().is_empty(),
            "consultant {} system prompt is empty",
            c.id
        );
    }
}

#[test]
fn routing_hints_rank_the_right_specialist_first() {
    let reg = ConsultantRegistry::from_embedded().unwrap();

    // A clear security task surfaces the Security specialist as the best hint.
    let picks: Vec<String> = reg
        .specialists_for("implement the oauth login flow with jwt tokens")
        .into_iter()
        .map(|c| c.id.to_string())
        .collect();
    assert_eq!(picks.first().map(String::as_str), Some("devon-carter"));

    // hint_matches is a lightweight containment check on the hints.
    let devon = reg.by_id("devon-carter").unwrap();
    assert!(devon.hint_matches("add oauth authentication"));
    assert!(!devon.hint_matches("style the homepage"));
}

#[test]
fn unknown_role_package_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("wizard.toml"),
        "[consultant]\nid = \"wiz-1\"\nname = \"Wiz\"\nrole = \"wizard\"\n",
    )
    .unwrap();

    let mut reg = ConsultantRegistry::from_embedded().unwrap();
    let err = reg
        .overlay_dir(dir.path())
        .expect_err("unknown role must be rejected");
    let msg = format!("{err:#}");
    assert!(msg.contains("wizard"), "error mentions the role: {msg}");
}

#[test]
fn missing_system_prompt_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("ghost.toml"),
        "[consultant]\nid = \"ghost-1\"\nname = \"Ghost\"\nrole = \"engineer\"\nsystem_prompt = \"prompts/nope.md\"\n",
    )
    .unwrap();

    let mut reg = ConsultantRegistry::from_embedded().unwrap();
    let err = reg
        .overlay_dir(dir.path())
        .expect_err("missing system prompt must be rejected");
    let msg = format!("{err:#}");
    assert!(msg.contains("nope.md"), "error names the prompt: {msg}");
}

#[test]
fn out_of_range_temperature_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("hot.toml"),
        "[consultant]\nid = \"hot-1\"\nname = \"Hot\"\nrole = \"engineer\"\n\n[consultant.model]\ntemperature = 3.5\n",
    )
    .unwrap();

    let mut reg = ConsultantRegistry::from_embedded().unwrap();
    let err = reg
        .overlay_dir(dir.path())
        .expect_err("temperature out of range must be rejected");
    let msg = format!("{err:#}");
    assert!(msg.contains("temperature"), "error names the field: {msg}");
}

#[test]
fn overlay_replaces_a_default_by_id_and_adds_a_new_specialist() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("prompts")).unwrap();
    // Override Marcus by reusing his id (new name, same role).
    std::fs::write(
        dir.path().join("marcus.toml"),
        "[consultant]\nid = \"marcus-reed\"\nname = \"Marcus Reed Jr\"\nrole = \"engineer\"\nsystem_prompt = \"prompts/marcus.md\"\n\n[consultant.routing]\nauto_join = true\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("prompts/marcus.md"), "override prompt").unwrap();
    // Add a brand-new specialist beyond the catalog (needs a NEW role? No — a
    // consultant must bind to a catalog role, so we add a second QA to show an
    // extra consultant in a role).
    std::fs::write(
        dir.path().join("extra.toml"),
        "[consultant]\nid = \"extra-1\"\nname = \"Extra\"\nrole = \"qa\"\n",
    )
    .unwrap();

    let mut reg = ConsultantRegistry::from_embedded().unwrap();
    let n = reg.overlay_dir(dir.path()).unwrap();
    assert_eq!(n, 2, "two user packages loaded");

    // Marcus's identity is overridden.
    let marcus = reg.by_id("marcus-reed").unwrap();
    assert_eq!(marcus.name, "Marcus Reed Jr");
    assert_eq!(marcus.system_prompt.as_deref(), Some("override prompt"));
    // The extra consultant joined the registry.
    assert!(reg.by_id("extra-1").is_some());
    // The roster grew past the embedded four.
    assert!(reg.count() >= 5);
}

#[test]
fn new_role_package_defines_and_exposes_a_role() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("compliance.toml"),
        "[consultant]\nid = \"compliance-1\"\nname = \"Priya Compliance\"\n\n[consultant.new_role]\nid = \"compliance\"\ntitle = \"Compliance Consultant\"\nscope = \"legal\"\n",
    )
    .unwrap();

    let mut reg = ConsultantRegistry::from_embedded().unwrap();
    assert_eq!(reg.overlay_dir(dir.path()).unwrap(), 1);

    // The new role is first-class in the dynamic role set.
    let role = reg
        .resolve_role("compliance")
        .expect("package-defined role resolves");
    assert_eq!(role.title, "Compliance Consultant");
    assert_eq!(role.scope, "legal");
    // It appears alongside the catalog roles, which stay intact.
    let ids: Vec<String> = reg.known_roles().into_iter().map(|r| r.id).collect();
    assert!(ids.contains(&"engineer".to_string()));
    assert!(ids.contains(&"compliance".to_string()));

    // The consultant is bound to its own role (not a catalog one).
    let c = reg.by_id("compliance-1").unwrap();
    assert_eq!(c.role, "compliance");
    assert_eq!(c.role_title, "Compliance Consultant");
    assert_eq!(c.scope, "legal");
}

#[test]
fn owner_can_hire_into_a_package_defined_role() {
    use casting::cursor::SqliteCursorStore;
    use casting::pm::AppState;
    use casting::projection::Projection;
    use casting::sqlite_store::SqliteEventStore;
    use futures::FutureExt;
    use std::sync::Arc;
    use tower::ServiceExt;

    // A registry carrying a brand-new role, not in the hardcoded catalog.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("compliance.toml"),
        "[consultant]\nid = \"compliance-1\"\nname = \"Priya Compliance\"\n\n[consultant.new_role]\nid = \"compliance\"\ntitle = \"Compliance Consultant\"\nscope = \"legal\"\n",
    )
    .unwrap();
    let mut reg = ConsultantRegistry::from_embedded().unwrap();
    reg.overlay_dir(dir.path()).unwrap();

    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    let state = AppState::new(store, cursors, "proj-newrole").with_consultants(Arc::new(reg));
    state
        .append(casting::event::Event::new(
            "proj-newrole",
            casting::event::Actor::System,
            casting::event::EventType::ProjectCreated,
            casting::event::Aggregate {
                kind: "project".into(),
                id: "proj-newrole".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();

    // Owner hires a compliance consultant through the real route.
    let app = casting::web::router(state.clone());
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/api/hire")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({ "role_id": "compliance" }).to_string(),
        ))
        .unwrap();
    let resp = app
        .oneshot(req)
        .now_or_never()
        .expect("dispatch should not block")
        .expect("infallible");
    assert_eq!(resp.status(), 200, "owner may hire a package-defined role");

    let proj = Projection::build(&state.store, "proj-newrole").unwrap();
    let hired = proj
        .agents
        .iter()
        .find(|a| a.role == "Compliance Consultant");
    assert!(
        hired.is_some(),
        "the new-role agent is hired: {:?}",
        proj.agents
    );
}
