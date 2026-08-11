//! Tests for the project registry (~/.casting/projects.json home-dir launcher).

use casting::registry::{ProjectEntry, Registry};

#[test]
fn empty_registry_is_default() {
    let reg = Registry::default();
    assert!(reg.projects.is_empty());
}

#[test]
fn register_lists_and_looks_up() {
    let mut reg = Registry::default();
    assert!(reg.register("demo".into(), "/tmp/demo".into()));
    // Upsert: second register with same name is an update, not a new entry.
    assert!(!reg.register("demo".into(), "/tmp/demo2".into()));

    assert_eq!(reg.projects.len(), 1);
    let e = reg.lookup("demo").expect("project present");
    assert_eq!(e.repo, std::path::PathBuf::from("/tmp/demo2"));
}

#[test]
fn register_multiple_preserves_order() {
    let mut reg = Registry::default();
    reg.register("a".into(), "/r/a".into());
    reg.register("b".into(), "/r/b".into());
    let names: Vec<&str> = reg.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b"]);
}

#[test]
fn remove_removes_only_matching() {
    let mut reg = Registry::default();
    reg.register("a".into(), "/r/a".into());
    reg.register("b".into(), "/r/b".into());
    assert!(reg.remove("a"));
    assert!(!reg.remove("a"), "already removed");
    assert_eq!(reg.projects.len(), 1);
    assert_eq!(reg.lookup("b").map(|p| p.name.as_str()), Some("b"));
}

#[test]
fn save_and_load_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("projects.json");

    let mut reg = Registry::default();
    reg.register("demo".into(), "/tmp/demo".into());
    reg.save(Some(&path)).unwrap();

    let loaded = Registry::load(Some(&path)).unwrap();
    assert_eq!(loaded, reg);
    assert_eq!(
        loaded.lookup("demo").map(|e| &e.repo),
        Some(&std::path::PathBuf::from("/tmp/demo"))
    );
}

#[test]
fn load_missing_file_is_empty_not_error() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("absent/projects.json");
    let reg = Registry::load(Some(&path)).unwrap();
    assert!(reg.projects.is_empty());
}

#[test]
fn project_entry_serde_shape() {
    let raw = r#"{"projects":[{"name":"demo","repo":"/tmp/demo"}]}"#;
    let reg: Registry = serde_json::from_str(raw).unwrap();
    assert_eq!(
        reg.projects[0],
        ProjectEntry {
            name: "demo".into(),
            repo: "/tmp/demo".into(),
        }
    );
}
