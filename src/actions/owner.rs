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

// --- Harness guards (2026-08-13, docs/plans/2026-08-13_harness-guards.md) ---

/// Build the owner-authored `BudgetSet` event — the hard spend circuit breaker.
/// `warn_at` is the fraction of `limit_usd` at which to warn (default 0.80);
/// at `limit_usd` the dispatch gate refuses all LLM calls. The breaker sits
/// OUTSIDE the PM's control, so only the owner (behind the owner bearer guard)
/// can set it.
pub fn owner_budget_set(project: &str, limit_usd: f64, warn_at: f64) -> Event {
    Event::new(
        project,
        Actor::Owner,
        EventType::BudgetSet,
        Aggregate {
            kind: "budget".into(),
            id: "budget".into(),
        },
        json!({ "limit_usd": limit_usd, "warn_at": warn_at }),
    )
}

/// Build the owner-authored `WorkPaused` event (manual pause of side-effecting
/// work). The liveness watchdog issues the same event as actor System.
pub fn owner_work_paused(project: &str, reason: &str) -> Event {
    Event::new(
        project,
        Actor::Owner,
        EventType::WorkPaused,
        Aggregate {
            kind: "guard".into(),
            id: "work-pause".into(),
        },
        json!({ "reason": reason, "by": "owner" }),
    )
}

/// Build the owner-authored `WorkResumed` event, clearing a `WorkPaused`.
/// NOTE: a BUDGET halt is derived from spend and is NOT cleared by this.
pub fn owner_work_resumed(project: &str) -> Event {
    Event::new(
        project,
        Actor::Owner,
        EventType::WorkResumed,
        Aggregate {
            kind: "guard".into(),
            id: "work-pause".into(),
        },
        json!({ "by": "owner" }),
    )
}
