//! Tests for the consultant registry + loader: curated embedded defaults,
//! default-cast resolution, strict validation, and user-directory overlays.

use casting::consultants::ConsultantRegistry;

#[test]
fn embedded_defaults_cover_all_assignable_catalog_roles() {
    let reg = ConsultantRegistry::from_embedded().expect("defaults parse + validate");
    // The default cast ships seven introduced consultants (the cast of 2026-08-14):
    // two SPECIAL non-assignable roles (pm, advisor) + five assignable ones.
    assert_eq!(reg.count(), 7);

    // Every assignable catalog role is bound to a consultant package.
    for role in [
        "lead-developer",
        "testing-engineer",
        "systems-architect",
        "stage-manager",
        "critic",
    ] {
        let c = reg
            .for_role(role)
            .unwrap_or_else(|| panic!("no consultant bound to role {role}"));
        assert_eq!(c.role, role);
        assert!(!c.name.is_empty(), "role {role} consultant has a name");
        assert!(
            !c.scope.is_empty(),
            "role {role} consultant carries a scope"
        );
        assert!(c.assignable, "role {role} must be assignable");
    }

    // The two SPECIAL roles ship as packages but bind to their OWN roles
    // (via new_role), carry `assignable = false`, and are never catalog roles.
    for id in ["pm", "advisor"] {
        let c = reg.by_id(id).unwrap_or_else(|| panic!("no {id} package"));
        assert!(!c.assignable, "{id} is a special, non-assignable role");
    }
}

#[test]
fn default_cast_is_the_seven_introduced_consultants() {
    let reg = ConsultantRegistry::from_embedded().unwrap();
    let default: Vec<String> = reg
        .default_cast()
        .into_iter()
        .map(|c| c.id.to_string())
        .collect();

    // All seven consultants are "introduced in the beginning" (auto_join).
    for id in [
        "pm",
        "advisor",
        "diego",
        "tess",
        "nina",
        "ali",
        "julien",
    ] {
        assert!(
            default.contains(&id.to_string()),
            "default cast must introduce {id}"
        );
    }
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

    // A testing-heavy task surfaces the Test Engineer as the best hint.
    let testing: Vec<String> = reg
        .specialists_for("write unit tests for the auth service covering empty arrays and timeouts")
        .into_iter()
        .map(|c| c.id.to_string())
        .collect();
    assert_eq!(testing.first().map(String::as_str), Some("tess"));

    // An adversarial security/scale review surfaces The Critic.
    let review: Vec<String> = reg
        .specialists_for(
            "review the payment endpoint for security at 10k requests and hostile input",
        )
        .into_iter()
        .map(|c| c.id.to_string())
        .collect();
    assert_eq!(review.first().map(String::as_str), Some("julien"));

    // hint_matches is a lightweight containment check on the hints.
    let architect = reg.by_id("nina").unwrap();
    assert!(architect.hint_matches("how should we structure the data layer for scale"));
    assert!(!architect.hint_matches("tweak the button color"));
}

#[test]
fn unknown_role_package_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("wizard.toml"),
        "[consultant]\nid = \"wiz-1\"\nname = \"Wiz\"\ncast_role = \"wizard\"\n",
    )
    .unwrap();

    let mut reg = ConsultantRegistry::from_embedded().unwrap();
    let err = reg
        .overlay_dir(dir.path())
        .expect_err("unknown cast_role must be rejected");
    let msg = format!("{err:#}");
    assert!(msg.contains("wizard"), "error mentions the role: {msg}");
}

#[test]
fn validates_seven_roles_on_startup() {
    // The embedded defaults have all 7 roles; this should pass.
    let reg = ConsultantRegistry::from_embedded().expect("all 7 roles present");
    assert_eq!(reg.count(), 7);

    // If one is missing, from_embedded fails.
    // We can't easily test the failure path from_embedded because the
    // embed is baked at compile time, so just verify the check succeeds.
    assert!(reg.validate_all_roles_present().is_ok());
}

#[test]
fn out_of_range_temperature_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("hot.toml"),
        "[consultant]\nid = \"hot-1\"\nname = \"Hot\"\ncast_role = \"stage_manager\"\n\n[consultant.model]\ntemperature = 3.5\n",
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
    // Override Diego by reusing his id (new name, same cast_role, inline prompt).
    std::fs::write(
        dir.path().join("diego.toml"),
        "[consultant]\nid = \"diego\"\nname = \"Diego Developer\"\ncast_role = \"lead_developer\"\nsystem_prompt = \"override prompt\"\n\n[consultant.routing]\nauto_join = true\n",
    )
    .unwrap();
    // Add a brand-new specialist beyond the default 7 (needs a CastRole too).
    std::fs::write(
        dir.path().join("extra.toml"),
        "[consultant]\nid = \"extra-1\"\nname = \"Extra\"\ncast_role = \"stage_manager\"\n",
    )
    .unwrap();

    let mut reg = ConsultantRegistry::from_embedded().unwrap();
    let n = reg.overlay_dir(dir.path()).unwrap();
    assert_eq!(n, 2, "two user packages loaded");

    // Diego's identity is overridden.
    let diego = reg.by_id("diego").unwrap();
    assert_eq!(diego.name, "Diego Developer");
    assert_eq!(diego.system_prompt.as_deref(), Some("override prompt"));
    // The extra consultant joined the registry.
    assert!(reg.by_id("extra-1").is_some());
    // The roster grew past the embedded seven.
    assert!(reg.count() >= 8);
}
