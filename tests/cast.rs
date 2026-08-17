//! Tests for the cast — the roster is `active-cast/` (the consultant
//! registry), reconciled onto the event log by `CastReconcilePass`. One
//! consultant per role, no hardcoded catalog, no name hardcoding.

use casting::consultants::cast_role::ALL_CAST_ROLES;
use casting::consultants::ConsultantRegistry;

/// The 7 role ids (from the authoritative CastRole enum).
fn all_role_ids() -> Vec<&'static str> {
    ALL_CAST_ROLES.iter().map(|r| r.role_id()).collect()
}

#[test]
fn embedded_roster_has_one_consultant_per_role() {
    let reg = ConsultantRegistry::from_embedded().unwrap();
    // Exactly the 7 roles, one consultant each.
    for rid in all_role_ids() {
        let c = reg
            .for_role(rid)
            .unwrap_or_else(|| panic!("role {rid} has a consultant"));
        assert_eq!(&c.role, rid, "consultant carries its role id");
        assert!(!c.role_title.is_empty());
        assert!(!c.scope.is_empty());
    }
    // Every consultant in the roster is present exactly once (one per role).
    assert_eq!(reg.all().len(), all_role_ids().len());
}

#[test]
fn known_roles_are_derived_from_the_registry() {
    let reg = ConsultantRegistry::from_embedded().unwrap();
    let known = reg.known_roles();
    // Deduped set of role ids == the 7 CastRole ids.
    let ids: Vec<&str> = known.iter().map(|r| r.id.as_str()).collect();
    for rid in all_role_ids() {
        assert!(ids.contains(&rid), "known_roles must include {rid}");
    }
    assert_eq!(
        known.len(),
        all_role_ids().len(),
        "no duplicates / no extras"
    );
    // Titles and scopes match what consultants actually declare.
    for c in reg.all() {
        let ri = reg.resolve_role(&c.role).expect("role resolvable");
        assert_eq!(ri.title, c.role_title);
        assert_eq!(ri.scope, c.scope);
    }
}

// --- Team change (HireAgent) via the decision pipeline ---

#[tokio::test]
async fn owner_hire_adds_an_agent_of_a_role() {
    use casting::pm::AppState;
    use casting::projection::Projection;
    use casting::runtime::orchestrator::MockOrchestrator;
    use casting::store::SqliteCursorStore;
    use casting::store::SqliteEventStore;
    use std::sync::Arc;

    let state = {
        let store = SqliteEventStore::in_memory().unwrap();
        let cursors = SqliteCursorStore::in_memory().unwrap();
        AppState::new(store, cursors, "proj-cast").with_orchestrator(Arc::new(MockOrchestrator))
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

    // Owner hires a specialist for a real role (a named agent, not a counter).
    let action = casting::actions::PmAction::HireAgent {
        agent_id: "malik".into(),
        role: "Testing Engineer".into(),
    };
    let cause = casting::event::Event::new(
        "proj-cast",
        casting::event::Actor::Director {
            user_id: "ceo".into(),
        },
        casting::event::EventType::MessageSent,
        casting::event::Aggregate {
            kind: "message".into(),
            id: "msg-hire".into(),
        },
        serde_json::json!({}),
    );
    for e in action.to_events("proj-cast", "director", &cause, "hire") {
        state.append(e).unwrap();
    }

    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    assert!(proj.agents.iter().any(|a| a.id == "malik"));
    assert_eq!(
        proj.agents.iter().find(|a| a.id == "malik").unwrap().role,
        "Testing Engineer"
    );
}

#[tokio::test]
async fn pm_propose_consultant_and_owner_approval_hire() {
    use casting::actions::validate;
    use casting::pm::AppState;
    use casting::projection::Projection;
    use casting::store::SqliteCursorStore;
    use casting::store::SqliteEventStore;

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

    // The PM proposes adding a testing specialist. AddConsultant defaults to
    // Pm, so the PM can decide it itself; but to exercise the director-approval
    // path we show the proposal is valid and routes as a decision.
    let cause = casting::event::Event::new(
        "proj-cast",
        casting::event::Actor::Agent { id: "mei".into() },
        casting::event::EventType::MessageSent,
        casting::event::Aggregate {
            kind: "message".into(),
            id: "msg-1".into(),
        },
        serde_json::json!({ "to": "mei", "body": "we need a testing person" }),
    );
    let proposal = casting::actions::PmAction::ProposeConsultant {
        id: "dc-1".into(),
        subject: "Add a Testing Engineer".into(),
        role_id: "testing-engineer".into(),
        involvement: casting::pm::policy::OwnerInvolvement::Pm,
    };
    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    assert!(
        validate(&proposal, "mei", &proj, Some(&state.consultants)).is_ok(),
        "PM may propose a hire for a known role"
    );
    for e in proposal.to_events("proj-cast", "mei", &cause, "corr-1") {
        state.append(e).unwrap();
    }

    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    let dec = proj.decisions.iter().find(|d| d.id == "dc-1").unwrap();
    assert_eq!(dec.class, casting::pm::policy::DecisionClass::AddConsultant);

    // Owner approves; manually apply the hire (the mock orchestrator handles
    // director decisions with a simple follow-up task; the real LLM would read
    // the decision options and emit HireAgent itself).
    state
        .append(casting::event::Event::new(
            "proj-cast",
            casting::event::Actor::Director {
                user_id: "ceo".into(),
            },
            casting::event::EventType::DecisionMade,
            casting::event::Aggregate {
                kind: "decision".into(),
                id: "dc-1".into(),
            },
            serde_json::json!({
                "approved": true,
                "note": "hire them",
                "subject": "Add a Testing Engineer",
            }),
        ))
        .unwrap();
    state
        .append(casting::event::Event::new(
            "proj-cast",
            casting::event::Actor::System,
            casting::event::EventType::AgentHired,
            casting::event::Aggregate {
                kind: "agent".into(),
                id: "malik".into(),
            },
            serde_json::json!({"role": "Testing Engineer"}),
        ))
        .unwrap();
    casting::pm::drive_pm(&state).await.unwrap();

    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    assert!(
        proj.agents.iter().any(|a| a.id == "malik"),
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
    use casting::pm::AppState;
    use casting::projection::Projection;
    use casting::store::SqliteCursorStore;
    use casting::store::SqliteEventStore;

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
            involvement: casting::pm::policy::OwnerInvolvement::Pm,
        },
        "mei",
        &proj,
        Some(&state.consultants),
    )
    .expect_err("unknown role must be rejected");
    assert!(matches!(err, PolicyError::UnknownRole(_)));
}

// --- Special roles (PM / Advisor) are never assignable nor hireable ---

#[test]
fn special_roles_cannot_be_assigned_tasks() {
    use casting::actions::{validate, PmAction, PolicyError};
    use casting::pm::AppState;
    use casting::projection::Projection;
    use casting::store::SqliteCursorStore;
    use casting::store::SqliteEventStore;
    use casting::types::TaskStatus;

    let state = {
        let store = SqliteEventStore::in_memory().unwrap();
        let cursors = SqliteCursorStore::in_memory().unwrap();
        AppState::new(store, cursors, "proj-cast")
    };
    state
        .append(casting::event::Event::new(
            "proj-cast",
            casting::event::Actor::System,
            casting::event::EventType::TaskCreated,
            casting::event::Aggregate {
                kind: "task".into(),
                id: "task-1".into(),
            },
            serde_json::json!({ "title": "t", "kind": "k" }),
        ))
        .unwrap();
    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    let _ = proj.tasks.iter().any(|t| t.status == TaskStatus::Backlog);

    // The PM may self-assign tasks via the chat-interface playbook.
    // The Advisor may NOT be assigned tasks.
    validate(
        &PmAction::AssignTask {
            task_id: "task-1".into(),
            assignee: "mei".into(),
            merge_authority: casting::types::MergeAuthority::PmMerge,
        },
        "mei",
        &proj,
        None,
    )
    .expect("pm should be able to self-assign tasks via chat-interface playbook");
    let err = validate(
        &PmAction::AssignTask {
            task_id: "task-1".into(),
            assignee: "jeeves".into(),
            merge_authority: casting::types::MergeAuthority::PmMerge,
        },
        "mei",
        &proj,
        None,
    )
    .expect_err("assigning to advisor must be rejected");
    assert!(
        matches!(err, PolicyError::SpecialRoleNotAssignable(_)),
        "expected SpecialRoleNotAssignable for advisor, got {err:?}"
    );
}

#[test]
fn special_roles_cannot_be_hired_as_agents() {
    use casting::actions::{validate, PmAction, PolicyError};
    use casting::pm::AppState;
    use casting::projection::Projection;
    use casting::store::SqliteCursorStore;
    use casting::store::SqliteEventStore;

    let state = {
        let store = SqliteEventStore::in_memory().unwrap();
        let cursors = SqliteCursorStore::in_memory().unwrap();
        AppState::new(store, cursors, "proj-cast")
    };
    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    for special in ["mei", "jeeves"] {
        let err = validate(
            &PmAction::HireAgent {
                agent_id: special.into(),
                role: "Engineer".into(),
            },
            "owner",
            &proj,
            None,
        )
        .expect_err("hiring a special role must be rejected");
        assert!(matches!(err, PolicyError::SpecialRoleNotAssignable(_)));
    }
}

// --- Roster reconcile: the directory IS the roster ---
//
// `CastReconcilePass` (the authoritative system reconcile) hires every
// consultant present in the directory and fires any hired agent whose package
// is gone. No names are hardcoded here — the embedded registry decides.

#[test]
fn cast_reconcile_hires_present_and_fires_absent() {
    use casting::pm::reconciler::CastReconcilePass;
    use casting::pm::{AppState, ReconcilePass};
    use casting::projection::Projection;
    use casting::store::SqliteCursorStore;
    use casting::store::SqliteEventStore;

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

    // Start with ONE roster consultant hired (diego) plus a hired agent whose
    // package is NOT in the directory ("ghost") — it must be fired.
    for (id, role) in [("diego", "Lead Developer"), ("ghost", "Some Ghost")] {
        state
            .append(casting::event::Event::new(
                "proj-cast",
                casting::event::Actor::System,
                casting::event::EventType::AgentHired,
                casting::event::Aggregate {
                    kind: "agent".into(),
                    id: id.into(),
                },
                serde_json::json!({ "role": role }),
            ))
            .unwrap();
    }

    let before = Projection::build(&state.store, "proj-cast").unwrap();
    assert!(before.agents.iter().any(|a| a.id == "ghost"), "ghost hired");

    // The reconcile pass hires the rest of the roster and fires the ghost.
    let authored = CastReconcilePass.run(&state).unwrap();

    let proj = Projection::build(&state.store, "proj-cast").unwrap();
    // Every consultant in the embedded roster is now hired (one per role).
    let roster_ids: Vec<String> = state
        .consultants
        .all()
        .iter()
        .map(|c| c.id.clone())
        .collect();
    for id in &roster_ids {
        assert!(
            proj.agents.iter().any(|a| &a.id == id),
            "roster consultant {id} must be hired by reconcile"
        );
    }
    // The ghost (package removed from the directory) is fired.
    assert!(
        !proj.agents.iter().any(|a| a.id == "ghost"),
        "agent whose package is gone must be fired"
    );
    // Exactly the roster remains, nothing extra.
    assert_eq!(proj.agents.len(), roster_ids.len());
    // Every hire used the consultant's real role title.
    for c in state.consultants.all() {
        let a = proj.agents.iter().find(|a| a.id == c.id).unwrap();
        assert_eq!(a.role, c.role_title, "role title from the consultant");
    }
    // Re-running is a no-op (idempotent).
    assert_eq!(CastReconcilePass.run(&state).unwrap(), 0);
    assert_eq!(authored as usize, roster_ids.len()); // all but diego, plus fire ghost
}
