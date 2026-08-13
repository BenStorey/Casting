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
