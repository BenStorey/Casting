//! Write-time event-stream integrity (roadmap item 4 hardening / option B).
//!
//! `cast log --verify` reports drift but is *advisory*. This module enforces
//! the core *precondition* invariants at the moment of write: certain events
//! may only be appended if their prerequisite already exists in the project's
//! history for the same aggregate (e.g. a `DecisionMade` must follow a
//! `DecisionProposed`; a `TaskCompleted` must follow a `TaskCreated`).
//!
//! It is **opt-in** (`AppState::with_integrity`) so existing fixtures and tests
//! that hand-append bare events keep working; production enables it. The store
//! stays generic and never encodes domain rules.

use crate::event::{Event, EventType};
use crate::projection::Projection;
use anyhow::{bail, Result};

/// The precondition each derived event type requires (for its aggregate id).
/// If `Some(req)`, appending the event requires a prior `req` in the projection.
fn precondition(event: &Event) -> Option<EventType> {
    use EventType::*;
    Some(match event.event_type {
        TaskStarted | TaskBlocked | TaskCompleted | TaskReadyForReview | TaskReviewed
        | TaskAssigned => TaskCreated,
        DecisionMade | DecisionSuperseded => DecisionProposed,
        _ => return None,
    })
}

/// Check that `event` may be appended given the current `proj`ection.
/// Returns Ok if no precondition applies or it is satisfied; otherwise an
/// error describing the missing precondition.
pub fn check_append(proj: &Projection, event: &Event) -> Result<()> {
    let Some(req) = precondition(event) else {
        return Ok(());
    };
    let seen = match event.event_type {
        EventType::TaskAssigned => proj.tasks.iter().any(|t| t.id == event.aggregate.id),
        _ => match req {
            EventType::TaskCreated => proj.tasks.iter().any(|t| t.id == event.aggregate.id),
            EventType::DecisionProposed => {
                proj.decisions.iter().any(|d| d.id == event.aggregate.id)
            }
            _ => false,
        },
    };
    if !seen {
        bail!(
            "cannot append {:?} for {}:{}: no prior {:?} in history",
            event.event_type,
            event.aggregate.kind,
            event.aggregate.id,
            req
        );
    }
    Ok(())
}
