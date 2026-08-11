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

// --- Team change (AddConsultant) via the decision pipeline ---

#[tokio::test]
async fn owner_hire_adds_an_agent_of_a_catalog_role() {
    use casting::cursor::SqliteCursorStore;
    use casting::pm::AppState;
    use casting::projection::Projection;
    use casting::sqlite_store::SqliteEventStore;

    let state = {
        let store = SqliteEventStore::in_memory().unwrap();
        let cursors = SqliteCursorStore::in_memory().unwrap();
        AppState::new(store, cursors, "proj-cast")
    };
    state
        .append(casting::event::Event::new(
            "proj-cast",
            casting::event::Actor::System,
            casting::event::EventType::ProjectCreated,
            casting::event::Aggregate {
                kind: "project".into(),
                id: "proj-cast".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();

    // Owner hires a security engineer (a catalog role, not in the default cast).
    let action = casting::actions::PmAction::HireAgent {
        agent_id: "security-1".into(),
        role: "Security Engineer".into(),
    };
    let cause = casting::event::Event::new(
        "proj-cast",
        casting::event::Actor::Owner,
        casting::event::EventType::MessageSent,
        casting::event::Aggregate {
            kind: "message".into(),
            id: "msg-hire".into(),
        },
        serde_json::json!({}),
    );
    for e in action.to_events("proj-cast", "owner", &cause, "hire") {
        state.append(e).unwrap();
    }

    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    assert!(proj.agents.iter().any(|a| a.id == "security-1"));
    assert_eq!(
        proj.agents
            .iter()
            .find(|a| a.id == "security-1")
            .unwrap()
            .role,
        "Security Engineer"
    );
}

#[tokio::test]
async fn pm_propose_consultant_and_owner_approval_hire() {
    use casting::actions::validate;
    use casting::cursor::SqliteCursorStore;
    use casting::pm::AppState;
    use casting::projection::Projection;
    use casting::sqlite_store::SqliteEventStore;

    let state = {
        let store = SqliteEventStore::in_memory().unwrap();
        let cursors = SqliteCursorStore::in_memory().unwrap();
        AppState::new(store, cursors, "proj-cast")
    };
    state
        .append(casting::event::Event::new(
            "proj-cast",
            casting::event::Actor::System,
            casting::event::EventType::ProjectCreated,
            casting::event::Aggregate {
                kind: "project".into(),
                id: "proj-cast".into(),
            },
            serde_json::json!({}),
        ))
        .unwrap();

    // The PM proposes adding a devops consultant. AddConsultant defaults to Pm,
    // so the PM can decide it itself; but to exercise the owner-approval path
    // we show the proposal is valid and routes as a decision.
    let cause = casting::event::Event::new(
        "proj-cast",
        casting::event::Actor::Agent { id: "pm".into() },
        casting::event::EventType::MessageSent,
        casting::event::Aggregate {
            kind: "message".into(),
            id: "msg-1".into(),
        },
        serde_json::json!({ "to": "pm", "body": "we need a devops person" }),
    );
    let proposal = casting::actions::PmAction::ProposeConsultant {
        id: "dc-1".into(),
        subject: "Add a DevOps consultant".into(),
        role_id: "devops".into(),
        involvement: casting::policy::OwnerInvolvement::Pm,
    };
    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    assert!(
        validate(&proposal, "pm", &proj).is_ok(),
        "PM may propose a hire"
    );
    for e in proposal.to_events("proj-cast", "pm", &cause, "corr-1") {
        state.append(e).unwrap();
    }

    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    let dec = proj.decisions.iter().find(|d| d.id == "dc-1").unwrap();
    assert_eq!(dec.class, casting::policy::DecisionClass::AddConsultant);

    // Owner approves; the hire is applied.
    state
        .append(casting::event::Event::new(
            "proj-cast",
            casting::event::Actor::Owner,
            casting::event::EventType::DecisionMade,
            casting::event::Aggregate {
                kind: "decision".into(),
                id: "dc-1".into(),
            },
            serde_json::json!({
                "approved": true,
                "note": "hire them",
                "subject": "Add a DevOps consultant",
            }),
        ))
        .unwrap();
    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    assert!(
        proj.agents.iter().any(|a| a.id == "devops-1"),
        "approved AddConsultant hire creates the agent: {:?}",
        proj.agents
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn unknown_role_proposal_is_rejected() {
    use casting::actions::{validate, PmAction, PolicyError};
    use casting::cursor::SqliteCursorStore;
    use casting::pm::AppState;
    use casting::projection::Projection;
    use casting::sqlite_store::SqliteEventStore;

    let state = {
        let store = SqliteEventStore::in_memory().unwrap();
        let cursors = SqliteCursorStore::in_memory().unwrap();
        AppState::new(store, cursors, "proj-cast")
    };
    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    let err = validate(
        &PmAction::ProposeConsultant {
            id: "dc-bad".into(),
            subject: "x".into(),
            role_id: "wizard".into(),
            involvement: casting::policy::OwnerInvolvement::Pm,
        },
        "pm",
        &proj,
    )
    .expect_err("unknown role must be rejected");
    assert!(matches!(err, PolicyError::UnknownRole(_)));
}
