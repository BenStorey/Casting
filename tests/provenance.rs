//! Integration tests for the provenance graph (Git slice increment 4).
//!
//! Provenance answers "why does this code exist?" by walking the event log:
//!   commit → changeSet → task → requirement → decision → director intent
//! (ADDENDUM §24–25). These tests build a realistic event chain (director message
//! → PM creates requirement + task → git branch + commit observed) and verify
//! the provenance query functions can walk it in both directions.

use casting::event::{Actor, Aggregate, Event, EventType, Metadata};
use casting::projection::Projection;
use casting::store::EventStore;
use casting::store::SqliteCursorStore;
use casting::store::SqliteEventStore;
use casting::workspace::git_observer;
use casting::workspace::provenance;
/// One-shot env var set so the git observer debounce doesn't fire during tests.
fn _init_debounce() {
    std::env::set_var("CAST_GIT_DEBOUNCE_MS", "0");
}

use casting::workspace::{Selfhost, Workspace};

/// A fresh workspace with a real git repo + an event store.
fn ws_with_repo() -> (
    tempfile::TempDir,
    Workspace,
    SqliteEventStore,
    SqliteCursorStore,
) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let ws = Workspace::open(&repo, Selfhost::Disabled).unwrap();
    ws.ensure_repo().unwrap();

    let store = SqliteEventStore::open(ws.state_dir.join("events.db")).unwrap();
    let cursors = SqliteCursorStore::open(ws.state_dir.join("cursors.db")).unwrap();

    (tmp, ws, store, cursors)
}

/// Append an event with provenance metadata (correlation_id + causation_id).
#[allow(clippy::too_many_arguments)]
fn append_with_meta(
    store: &SqliteEventStore,
    project: &str,
    actor: Actor,
    event_type: EventType,
    agg: Aggregate,
    data: serde_json::Value,
    correlation: &str,
    causation: Option<uuid::Uuid>,
) -> Event {
    let mut ev = Event::new(project, actor, event_type, agg, data);
    ev.metadata = Metadata {
        correlation_id: Some(correlation.to_string()),
        causation_id: causation,
        agent_run_id: Some(format!("sim-run-{correlation}")),
    };
    store.append(ev).unwrap()
}

#[test]
fn provenance_traces_commit_to_owner_message() {
    _init_debounce();
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;
    let project = "proj";

    // 1. Owner sends a message.
    let owner_msg = store
        .append(Event::new(
            project,
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-director-1".into(),
            },
            serde_json::json!({ "to": "pm", "body": "Build me a todo app" }),
        ))
        .unwrap();

    // 2. PM creates a requirement + task, linked via correlation_id +
    //    causation_id to the director's message (same shape as the scripted PM).
    let correlation = "run-1";
    append_with_meta(
        &store,
        project,
        Actor::Agent { id: "pm".into() },
        EventType::RequirementCreated,
        Aggregate {
            kind: "requirement".into(),
            id: "req-1".into(),
        },
        serde_json::json!({ "title": "Build me a todo app", "description": "..." }),
        correlation,
        Some(owner_msg.event_id),
    );

    let _task_ev = append_with_meta(
        &store,
        project,
        Actor::Agent { id: "pm".into() },
        EventType::TaskCreated,
        Aggregate {
            kind: "task".into(),
            id: "task-501".into(),
        },
        serde_json::json!({ "title": "Implement todo app", "kind": "feature" }),
        correlation,
        Some(owner_msg.event_id),
    );

    // 3. Git: create a branch and commit, then observe.
    std::fs::write(ws.repo.join("README.md"), "hello\n").unwrap();
    ws.git_command().arg("add").arg(".").output().unwrap();
    ws.git_command()
        .arg("commit")
        .arg("-m")
        .arg("initial")
        .output()
        .unwrap();
    ws.git_command()
        .arg("checkout")
        .arg("-b")
        .arg("casting/task-501-todo")
        .output()
        .unwrap();
    std::fs::write(ws.repo.join("app.py"), "print('todo')\n").unwrap();
    ws.git_command().arg("add").arg(".").output().unwrap();
    ws.git_command()
        .arg("commit")
        .arg("-m")
        .arg("implement todo")
        .output()
        .unwrap();

    git_observer::observe(&ws, &store, &cursors, project).unwrap();

    // 4. Find the commit sha from the projection.
    let proj = Projection::build(&store, project).unwrap();
    let todo_commit = proj
        .commits
        .iter()
        .find(|c| c.message == "implement todo")
        .expect("the todo commit should be observed");
    let sha = &todo_commit.sha;

    // 5. Query the provenance chain for this commit.
    let chain = provenance::for_commit(&store, project, sha).unwrap();

    // The chain should have at least: CommitObserved → TaskCreated →
    // RequirementCreated → MessageSent (director).
    assert_eq!(chain.commit, *sha);
    assert_eq!(chain.task_id.as_deref(), Some("task-501"));
    assert_eq!(chain.changeset_id.as_deref(), Some("changeset-task-501"));
    assert_eq!(chain.requirement_id.as_deref(), Some("req-1"));

    // Verify the chain contains the expected links.
    let kinds: Vec<&str> = chain.chain.iter().map(|l| l.entity_kind.as_str()).collect();
    assert!(kinds.contains(&"commit"), "chain should include the commit");
    assert!(kinds.contains(&"task"), "chain should include the task");
    assert!(
        kinds.contains(&"requirement"),
        "chain should include the requirement"
    );
    assert!(
        kinds.contains(&"message"),
        "chain should include the director's message"
    );

    // the director message description should include the original body.
    assert!(
        chain
            .owner_message
            .as_deref()
            .unwrap_or_default()
            .contains("Build me a todo app"),
        "owner message should be in the chain: {:?}",
        chain.owner_message
    );
}

#[test]
fn provenance_for_task_traces_to_requirement_and_commits() {
    let (retain, ws, store, cursors) = ws_with_repo();
    let _ = retain;
    let project = "proj";

    // Build the same chain as above but test the reverse direction.
    let owner_msg = store
        .append(Event::new(
            project,
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-director-2".into(),
            },
            serde_json::json!({ "to": "pm", "body": "Build a REST API" }),
        ))
        .unwrap();

    let correlation = "run-2";
    append_with_meta(
        &store,
        project,
        Actor::Agent { id: "pm".into() },
        EventType::RequirementCreated,
        Aggregate {
            kind: "requirement".into(),
            id: "req-2".into(),
        },
        serde_json::json!({ "title": "Build a REST API", "description": "..." }),
        correlation,
        Some(owner_msg.event_id),
    );

    append_with_meta(
        &store,
        project,
        Actor::Agent { id: "pm".into() },
        EventType::TaskCreated,
        Aggregate {
            kind: "task".into(),
            id: "task-502".into(),
        },
        serde_json::json!({ "title": "Implement REST API", "kind": "feature" }),
        correlation,
        Some(owner_msg.event_id),
    );

    // Git: branch + commit.
    std::fs::write(ws.repo.join("README.md"), "hello\n").unwrap();
    ws.git_command().arg("add").arg(".").output().unwrap();
    ws.git_command()
        .arg("commit")
        .arg("-m")
        .arg("initial")
        .output()
        .unwrap();
    ws.git_command()
        .arg("checkout")
        .arg("-b")
        .arg("casting/task-502-api")
        .output()
        .unwrap();
    std::fs::write(ws.repo.join("api.py"), "print('api')\n").unwrap();
    ws.git_command().arg("add").arg(".").output().unwrap();
    ws.git_command()
        .arg("commit")
        .arg("-m")
        .arg("implement api")
        .output()
        .unwrap();

    git_observer::observe(&ws, &store, &cursors, project).unwrap();

    // Query provenance for the task (reverse direction).
    let task_prov = provenance::for_task(&store, project, "task-502").unwrap();

    assert_eq!(task_prov.task_id, "task-502");
    assert_eq!(
        task_prov.changeset_id.as_deref(),
        Some("changeset-task-502")
    );
    assert_eq!(task_prov.requirement_id.as_deref(), Some("req-2"));
    assert!(
        task_prov
            .owner_message
            .as_deref()
            .unwrap_or_default()
            .contains("Build a REST API"),
        "owner message should be traced: {:?}",
        task_prov.owner_message
    );
    assert!(
        !task_prov.commits.is_empty(),
        "commits should be linked to the task"
    );
    assert_eq!(
        task_prov.branch.as_deref(),
        Some("casting/task-502-api"),
        "branch should be traced"
    );
}

#[test]
fn provenance_returns_empty_chain_for_unknown_commit() {
    let (retain, _ws, store, _cursors) = ws_with_repo();
    let _ = retain;
    let chain = provenance::for_commit(&store, "proj", "nonexistent-sha").unwrap();
    assert_eq!(chain.commit, "nonexistent-sha");
    assert!(chain.task_id.is_none());
    assert!(chain.chain.is_empty());
}

// --- Decision audit (state-core maturity step 1) ---

fn make_proj_store() -> (SqliteEventStore, SqliteCursorStore) {
    let store = SqliteEventStore::in_memory().unwrap();
    let cursors = SqliteCursorStore::in_memory().unwrap();
    (store, cursors)
}

fn appends_owner_message(store: &SqliteEventStore, body: &str) -> Event {
    store
        .append(Event::new(
            "proj",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "msg-dependent".into(),
            },
            serde_json::json!({ "to": "pm", "body": body }),
        ))
        .unwrap()
}

/// Append a decision proposal causally linked to `cause` (director message), so
/// the audit can trace back to it.
fn append_proposal(store: &SqliteEventStore, id: &str, subject: &str, cause: &Event) -> Event {
    let meta = Metadata {
        causation_id: Some(cause.event_id),
        correlation_id: Some("corr-audit".into()),
        ..Default::default()
    };
    let mut ev = Event::new(
        "proj",
        Actor::Agent { id: "pm".into() },
        EventType::DecisionProposed,
        Aggregate {
            kind: "decision".into(),
            id: id.into(),
        },
        serde_json::json!({
            "subject": subject,
            "options": serde_json::json!({}),
            "recommendation": "A",
            "class": "database",
            "involvement": "ask",
        }),
    );
    ev.metadata = meta;
    store.append(ev).unwrap()
}

#[test]
fn decision_audit_traces_to_owner_message_when_proposed_only() {
    let (store, _c) = make_proj_store();
    let msg = appends_owner_message(&store, "Build a thing");
    append_proposal(&store, "decision-1", "Database choice", &msg);

    let audit = provenance::for_decision(&store, "proj", "decision-1").unwrap();
    assert_eq!(audit.subject, "Database choice");
    assert_eq!(audit.class, casting::pm::policy::DecisionClass::Database);
    assert_eq!(
        audit.involvement,
        casting::pm::policy::OwnerInvolvement::Ask
    );
    assert_eq!(audit.status, "proposed");
    assert_eq!(audit.proposed_by, "pm");
    assert_eq!(audit.decided_by, None);
    assert_eq!(audit.owner_message.as_deref(), Some("Build a thing"));
    // Chain: proposal → director message.
    let kinds: Vec<&str> = audit.chain.iter().map(|l| l.entity_kind.as_str()).collect();
    assert!(kinds.contains(&"decision"));
    assert!(kinds.contains(&"message"));
}

#[test]
fn decision_audit_records_decider_and_note_when_decided() {
    let (store, _c) = make_proj_store();
    let msg = appends_owner_message(&store, "Build a thing");
    append_proposal(&store, "decision-2", "Database choice", &msg);

    // Owner approves it → DecisionMade (actor = Owner, note attached).
    store
        .append(Event::new(
            "proj",
            Actor::Director {
                user_id: "ceo".into(),
            },
            EventType::DecisionMade,
            Aggregate {
                kind: "decision".into(),
                id: "decision-2".into(),
            },
            serde_json::json!({ "approved": true, "note": "Postgres is fine" }),
        ))
        .unwrap();

    let audit = provenance::for_decision(&store, "proj", "decision-2").unwrap();
    assert_eq!(audit.status, "approved");
    assert_eq!(audit.decided_by.as_deref(), Some("director"));
    assert_eq!(audit.note.as_deref(), Some("Postgres is fine"));
    let kinds: Vec<&str> = audit.chain.iter().map(|l| l.event_type.as_str()).collect();
    assert!(kinds.contains(&"DecisionMade"));
}

#[test]
fn decision_audit_returns_empty_for_unknown_decision() {
    let (store, _c) = make_proj_store();
    let audit = provenance::for_decision(&store, "proj", "nope").unwrap();
    assert_eq!(audit.status, "unknown");
    assert!(audit.chain.is_empty());
}
