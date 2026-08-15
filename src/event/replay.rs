//! Event-stream replay + integrity verification (roadmap item 4).
//!
//! The event log is the ONLY authoritative history. `cast log` surfaces that
//! raw history for inspection (dump) and checks the stream's structural
//! invariants (verify). These are read-only over the store — they never mutate
//! and never pretend to be authoritative themselves.
//!
//! Invariants verified:
//!   - sequence is contiguous from 1..max (no gaps),
//!   - a DecisionMade always follows a DecisionProposed for the same decision,
//!   - a TaskCompleted always follows a TaskCreated for the same task.

use crate::event::{Event, EventType};
use crate::store::EventStore;
use anyhow::Result;

/// Human-readable dump of a project's event stream, one line per event:
/// `#seq  event_type  aggregate_kind:aggregate_id  actor  data`.
pub fn dump<S: EventStore>(store: &S, project: &str) -> Result<Vec<String>> {
    let events = store.read_since(project, 0)?;
    Ok(events
        .iter()
        .map(|e| {
            format!(
                "#{:>4}  {:<26} {}:{}  {}  {}",
                e.sequence,
                format!("{:?}", e.event_type),
                e.aggregate.kind,
                e.aggregate.id,
                actor_label(&e.actor),
                e.data,
            )
        })
        .collect())
}

/// Verify a project's event stream invariants. Returns a list of problems
/// (empty = clean). Advisory: reports drift rather than blocking appends.
pub fn verify<S: EventStore>(store: &S, project: &str) -> Result<Vec<String>> {
    let events = store.read_since(project, 0)?;
    let mut problems = Vec::new();

    // 1. Sequence contiguous from 1..max (no gaps, no dups).
    {
        let max = store.latest_sequence(project)?;
        let mut seqs: Vec<i64> = events.iter().map(|e| e.sequence).collect();
        seqs.sort();
        seqs.dedup();
        if seqs.len() != max as usize {
            problems.push(format!(
                "sequence gap/dup: {} distinct sequences present, expected {} (1..{max})",
                seqs.len(),
                max
            ));
        } else {
            for (i, s) in seqs.iter().enumerate() {
                if *s != i as i64 + 1 {
                    problems.push(format!(
                        "sequence is not contiguous: expected {}, got {s}",
                        i + 1
                    ));
                    break;
                }
            }
        }
    }

    // 2. Precondition indices: a DecisionMade requires a prior DecisionProposed
    // for the same aggregate id; a TaskCompleted requires a prior TaskCreated.
    check_precondition(
        &events,
        &mut problems,
        EventType::DecisionMade,
        EventType::DecisionProposed,
        "DecisionMade without a prior DecisionProposed",
    );
    check_precondition(
        &events,
        &mut problems,
        EventType::TaskCompleted,
        EventType::TaskCreated,
        "TaskCompleted without a prior TaskCreated",
    );

    Ok(problems)
}

/// For every event of `derived` kind, ensure its aggregate id had an earlier
/// event of `precondition` kind.
fn check_precondition(
    events: &[Event],
    problems: &mut Vec<String>,
    derived: EventType,
    precondition: EventType,
    msg: &str,
) {
    let mut seen_precondition: Vec<&str> = Vec::new();
    for e in events {
        if e.event_type == precondition {
            if !seen_precondition.contains(&e.aggregate.id.as_str()) {
                seen_precondition.push(&e.aggregate.id);
            }
        } else if e.event_type == derived && !seen_precondition.contains(&e.aggregate.id.as_str()) {
            problems.push(format!(
                "#{} {msg}: aggregate {} ({})",
                e.sequence, e.aggregate.id, e.aggregate.kind
            ));
        }
    }
}

fn actor_label(a: &crate::event::Actor) -> String {
    match a {
        crate::event::Actor::Owner => "owner".into(),
        crate::event::Actor::Agent { id } => id.clone(),
        crate::event::Actor::System => "system".into(),
    }
}
