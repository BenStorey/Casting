//! Tests for the cast — role catalog + default cast (+ TeamChange policy class
//! integration). The cast is configuration/data, never authoritative state.

use casting::cast::{role_by_id, role_by_title, DEFAULT_CAST, ROLE_CATALOG};

#[test]
fn catalog_has_sane_roles_with_scopes() {
    let ids: Vec<&str> = ROLE_CATALOG.iter().map(|r| r.id).collect();
    assert!(ids.contains(&"engineer"));
    assert!(ids.contains(&"qa"));
    // Every role has a scope (governance area).
    for r in ROLE_CATALOG {
        assert!(!r.scope.is_empty());
    }
}

#[test]
fn role_lookup_by_id_and_title() {
    let eng = role_by_id("engineer").expect("engineer role exists");
    assert_eq!(eng.scope, "engineering");
    // role_by_title finds it by its stored title.
    assert_eq!(role_by_title("Engineer").unwrap().id, "engineer");
    // Unknown ids/titles return None.
    assert!(role_by_id("nope").is_none());
    assert!(role_by_title("Nope").is_none());
}

#[test]
fn default_cast_members_have_catalog_roles() {
    // Every default cast member resolves to a real catalog role with a scope.
    for m in DEFAULT_CAST {
        let role = role_by_id(m.role_id).unwrap_or_else(|| panic!("no role {}", m.role_id));
        assert!(!role.title.is_empty());
        assert!(!role.scope.is_empty());
    }
    assert_eq!(DEFAULT_CAST.len(), 2);
}
