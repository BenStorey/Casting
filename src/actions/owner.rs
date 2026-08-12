//! Owner-authored event shapes (review finding C: centralize event shapes here
//! so web.rs and to_events never drift).
use crate::event::{Actor, Aggregate, Event, EventType};
use serde_json::json;

/// Build the owner-authored `DecisionMade` event (review finding C: centralize
/// event shapes here so web.rs and to_events never drift). The owner resolves a
/// proposed decision; `subject` is the decision's subject for the owner's audit
/// trail, and `note` the verdict's rationale.
pub fn owner_decision_made(
    project: &str,
    decision_id: &str,
    subject: &str,
    approved: bool,
    note: Option<String>,
) -> Event {
    decision_made_event(Actor::Owner, project, decision_id, subject, approved, note)
}

/// The single shared `DecisionMade` builder. Both the owner-authored path
/// (`owner_decision_made`) and the generic action→event path (MakeDecision in
/// to_events) go through here so the event SHAPE can never drift between
/// PM/agent-made and owner-made decisions. `note` is emitted as a string field
/// (None => ""), which the reducer reads via string_field.
pub(crate) fn decision_made_event(
    actor: Actor,
    project: &str,
    decision_id: &str,
    subject: &str,
    approved: bool,
    note: Option<String>,
) -> Event {
    Event::new(
        project,
        actor,
        EventType::DecisionMade,
        Aggregate {
            kind: "decision".into(),
            id: decision_id.into(),
        },
        json!({
            "subject": subject,
            "approved": approved,
            "note": note.unwrap_or_default(),
        }),
    )
}

/// Build the owner-authored `DecisionPolicyChanged` event (owner configures the
/// owner-involvement required for a decision class).
pub fn owner_policy_changed(
    project: &str,
    class: crate::policy::DecisionClass,
    involvement: crate::policy::OwnerInvolvement,
) -> Event {
    Event::new(
        project,
        Actor::Owner,
        EventType::DecisionPolicyChanged,
        Aggregate {
            kind: "decision_policy".into(),
            id: format!("{class:?}"),
        },
        json!({
            "class": class,
            "involvement": involvement,
        }),
    )
}

/// Build the owner-authored `ProjectDirectiveCreated` event (owner sets
/// governance). `created_by` is hardcoded to the owner because a directive,
/// once created, is attributed to its author regardless of later security.
pub fn owner_directive_created(
    project: &str,
    id: &str,
    kind: crate::directive::DirectiveKind,
    statement: &str,
    scope: Vec<String>,
    strength: crate::directive::DirectiveStrength,
) -> Event {
    Event::new(
        project,
        Actor::Owner,
        EventType::ProjectDirectiveCreated,
        Aggregate {
            kind: "directive".into(),
            id: id.into(),
        },
        json!({
            "kind": kind,
            "statement": statement,
            "scope": scope,
            "strength": strength,
            "created_by": "owner",
            "supersedes": null,
        }),
    )
}
