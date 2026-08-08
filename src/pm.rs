//! Simulated Project Manager — the vertical slice's "company" control loop.
//!
//! Per docs/ADDENDUM.md the PM is a control loop over the event stream, not a
//! chatbot (§1), holds a durable cursor (§2), and turns owner input into
//! organizational work. This slice uses a deterministic *scripted* PM (D2:
//! scripted-first) that reacts to owner messages/decisions by appending a fixed
//! but real chain of domain events (`RequirementCreated → TaskCreated → … →
//! DecisionProposed → OwnerDecisionRecorded`), so the architecture is proven
//! before any LLM is wired in. A real provider can later produce the same
//! structured actions behind this same loop.
//!
//! Wake vs act: the loop wakes on a cheap notification (a broadcast of newly
//! appended events) and drains EVERYTHING since its cursor in one pass,
//! then advances the cursor — it never reasons per-event (docs/PM_INVOCATION_TRIGGERS.md).

use crate::cursor::CursorStore;
use crate::event::{Actor, Aggregate, Event, EventType, Metadata};
use crate::projection::Projection;
use crate::sqlite_store::SqliteEventStore;
use crate::store::EventStore;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

pub const PM_CONSUMER: &str = "pm";

/// Stable agent roster the simulated company uses.
const AGENT_PM: &str = "pm";
const AGENT_ENG: &str = "marcus-reed";
const AGENT_QA: &str = "maya-patel";

/// Shared runtime state: the event store, durable cursors, the active project,
/// and a broadcast channel for notifying subscribers (UI/SSE and the PM) that
/// events were appended. A notification is a *hint to consume persisted
/// events*, never the source of truth (brief §17).
#[derive(Clone)]
pub struct AppState {
    pub store: SqliteEventStore,
    pub cursors: CursorStore,
    pub project: String,
    events: Arc<broadcast::Sender<Event>>,
}

impl AppState {
    pub fn new(store: SqliteEventStore, cursors: CursorStore, project: impl Into<String>) -> Self {
        let (tx, _) = broadcast::channel(1024);
        AppState {
            store,
            cursors,
            project: project.into(),
            events: Arc::new(tx),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Append an event to the store, assign its sequence, then broadcast it to
    /// subscribers (a wake hint for the PM, a realtime push for the UI).
    pub fn append(&self, event: Event) -> Result<Event> {
        let stored = self.store.append(event)?;
        // Ignore send errors: nobody listening just means no one cares yet.
        let _ = self.events.send(stored.clone());
        Ok(stored)
    }
}

/// Spawn the simulated PM loop. It blocks on wake hints, drains all new events
/// since its cursor, lets the scripted policy respond, then advances the cursor.
pub async fn run_pm(state: AppState) {
    let mut rx = state.subscribe();
    loop {
        // Wake on any broadcast (cheap); a 500ms timeout is a safety poll so we
        // still catch events if broadcast lagged. Never per-event reasoning.
        let _ = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        if let Err(e) = drain(&state).await {
            eprintln!("[pm] drain error: {e:#}");
        }
    }
}

/// Run one drain pass now (test/CLI entry). Consumes all events since the PM's
/// cursor, lets the scripted policy respond, and advances the cursor. Returns
/// how many events the PM authored. Idempotent: a second call with no new
/// events returns 0.
pub async fn drive_pm(state: &AppState) -> Result<u32> {
    drain(state).await
}

/// Drain everything since the PM's cursor in one pass and respond, then commit
/// the new cursor position. Returns the number of events the PM appended.
async fn drain(state: &AppState) -> Result<u32> {
    let cursor = state.cursors.get(&state.project, PM_CONSUMER)?;
    let new_events = state.store.read_since(&state.project, cursor.last_seen)?;
    if new_events.is_empty() {
        return Ok(0);
    }

    let projection = Projection::build(&state.store, &state.project)?;
    let authored = respond(state, &projection, &new_events).await?;

    let latest = state.store.latest_sequence(&state.project)?;
    state.cursors.advance(&state.project, PM_CONSUMER, latest)?;
    Ok(authored)
}

/// The scripted policy: look at the events since the cursor and decide what the
/// PM "company" emits next. Deterministic for this slice.
async fn respond(state: &AppState, projection: &Projection, new_events: &[Event]) -> Result<u32> {
    let mut authored = 0u32;

    for e in new_events {
        let (is_owner_message, body) = match e.event_type {
            EventType::MessageSent if e.actor == Actor::Owner => (
                true,
                e.data.get("body").and_then(|b| b.as_str()).unwrap_or(""),
            ),
            _ => (false, ""),
        };

        if is_owner_message {
            let started = !projection.requirements.is_empty();
            authored += if started {
                script_acknowledge(state, e).await?
            } else {
                script_onboard(state, e, body).await?
            };
            continue;
        }

        if e.event_type == EventType::OwnerDecisionRecorded {
            authored += script_owner_decision(state, e).await?;
        }
    }

    Ok(authored)
}

/// Build metadata linking a new event to the one that caused it (the "why?"
/// chain — brief §11, addendum §24). Uses the owner's input event as the root of
/// its own correlation id.
fn linked(causation: &Event, correlation: &str) -> Metadata {
    Metadata {
        correlation_id: Some(correlation.to_string()),
        causation_id: Some(causation.event_id),
        agent_run_id: Some(format!("sim-run-{}", causation.sequence)),
    }
}

/// Make a domain event with provenance metadata already attached.
fn ev(
    project: &str,
    actor: Actor,
    id: &str,
    kind: &str,
    event_type: EventType,
    data: serde_json::Value,
    meta: Metadata,
) -> Event {
    let mut e = Event::new(
        project,
        actor,
        event_type,
        Aggregate {
            kind: kind.to_string(),
            id: id.to_string(),
        },
        data,
    );
    e.metadata = meta;
    e
}

/// Append a series of events one at a time with a short pause so the UI can
/// animate the company working (brief §35). Returns how many were appended.
async fn emit(state: &AppState, events: Vec<Event>) -> Result<u32> {
    let n = events.len() as u32;
    for e in events {
        state.append(e)?;
        tokio::time::sleep(Duration::from_millis(220)).await;
    }
    Ok(n)
}

/// First owner message: onboard the company and kick off a build.
async fn script_onboard(state: &AppState, cause: &Event, body: &str) -> Result<u32> {
    let project = state.project.clone();
    let title = if body.trim().is_empty() {
        "the product".to_string()
    } else {
        body.trim().to_string()
    };
    let corr = format!("req-{}", cause.event_id);

    let mut crew = Vec::new();
    // Requirement from owner intent.
    crew.push(ev(
        &project,
        Actor::System,
        "req-1",
        "requirement",
        EventType::RequirementCreated,
        serde_json::json!({"title": title.clone(), "description": body}),
        linked(cause, &corr),
    ));
    // Hire the team (idempotent-ish for slice one).
    crew.push(ev(
        &project,
        Actor::System,
        AGENT_ENG,
        "agent",
        EventType::AgentHired,
        serde_json::json!({"role": "Principal Engineer"}),
        linked(cause, &corr),
    ));
    crew.push(ev(
        &project,
        Actor::System,
        AGENT_QA,
        "agent",
        EventType::AgentHired,
        serde_json::json!({"role": "QA Consultant"}),
        linked(cause, &corr),
    ));
    // PM greets the owner.
    crew.push(ev(
        &project,
        Actor::Agent { id: AGENT_PM.into() },
        format!("msg-{}", cause.sequence).as_str(),
        "message",
        EventType::MessageSent,
        serde_json::json!({
            "to": "owner",
            "body": format!("Understood — \u{201c}{title}\u{201d}. I've broken this into tasks and brought in Marcus (engineering) and Maya (QA). Stand by.")
        }),
        linked(cause, &corr),
    ));
    emit(state, crew).await?;
    let mut n = 4;

    // Design task: assigned -> started -> completed.
    n += emit(
        state,
        vec![
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_PM.into(),
                },
                "task-design",
                "task",
                EventType::TaskCreated,
                serde_json::json!({"title": format!("Design {title}"), "kind": "feature"}),
                linked(cause, &corr),
            ),
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_PM.into(),
                },
                "task-design",
                "task",
                EventType::TaskAssigned,
                serde_json::json!({"assignee": AGENT_ENG}),
                linked(cause, &corr),
            ),
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_ENG.into(),
                },
                "task-design",
                "task",
                EventType::TaskStarted,
                serde_json::json!({}),
                linked(cause, &corr),
            ),
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_ENG.into(),
                },
                "task-design",
                "task",
                EventType::TaskCompleted,
                serde_json::json!({"result": format!("Designed {title}")}),
                linked(cause, &corr),
            ),
        ],
    )
    .await?;

    // Core implementation: create + assign + start + complete.
    n += emit(
        state,
        vec![
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_PM.into(),
                },
                "task-core",
                "task",
                EventType::TaskCreated,
                serde_json::json!({"title": format!("Implement {title} core"), "kind": "feature"}),
                linked(cause, &corr),
            ),
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_PM.into(),
                },
                "task-core",
                "task",
                EventType::TaskAssigned,
                serde_json::json!({"assignee": AGENT_ENG}),
                linked(cause, &corr),
            ),
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_ENG.into(),
                },
                "task-core",
                "task",
                EventType::TaskStarted,
                serde_json::json!({}),
                linked(cause, &corr),
            ),
        ],
    )
    .await?;

    // QA raises an informational observation (the feedback loop, brief §20).
    n += emit(
        state,
        vec![ev(
            &project,
            Actor::Agent {
                id: AGENT_QA.into(),
            },
            "obs-1",
            "observation",
            EventType::ObservationCreated,
            serde_json::json!({
                "severity": "info",
                "subject": "HTTPS not enabled in the scaffold",
                "body": "Noted during review. Won't fix now, but worth a task later.",
                "recommended_action": "Create a hardening task",
                "requires_owner": false,
                "pm_action_required": false
            }),
            linked(cause, &corr),
        )],
    )
    .await?;

    // Engineer finishes the core implementation.
    n += emit(
        state,
        vec![ev(
            &project,
            Actor::Agent {
                id: AGENT_ENG.into(),
            },
            "task-core",
            "task",
            EventType::TaskCompleted,
            serde_json::json!({"result": format!("Core implementation of {title} done")}),
            linked(cause, &corr),
        )],
    )
    .await?;

    // QA verifies with a test task.
    n += emit(
        state,
        vec![
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_PM.into(),
                },
                "task-qa",
                "task",
                EventType::TaskCreated,
                serde_json::json!({"title": "Set up automated tests", "kind": "feature"}),
                linked(cause, &corr),
            ),
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_PM.into(),
                },
                "task-qa",
                "task",
                EventType::TaskAssigned,
                serde_json::json!({"assignee": AGENT_QA}),
                linked(cause, &corr),
            ),
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_QA.into(),
                },
                "task-qa",
                "task",
                EventType::TaskStarted,
                serde_json::json!({}),
                linked(cause, &corr),
            ),
            ev(
                &project,
                Actor::Agent {
                    id: AGENT_QA.into(),
                },
                "task-qa",
                "task",
                EventType::TaskCompleted,
                serde_json::json!({"result": "Test suite passing"}),
                linked(cause, &corr),
            ),
        ],
    )
    .await?;

    // A decision the owner must make (delegated authority — brief §5, §21).
    n += emit(
        state,
        vec![
            ev(
                &project,
                Actor::Agent { id: AGENT_PM.into() },
                "decision-db",
                "decision",
                EventType::DecisionProposed,
                serde_json::json!({
                    "subject": "Database choice",
                    "options": {
                        "A": "PostgreSQL — robust, more infra, approx $18",
                        "B": "SQLite — dead simple, zero infra, approx $9"
                    },
                    "recommendation": "A",
                    "owner_involvement": "Required"
                }),
                linked(cause, &corr),
            ),
            ev(
                &project,
                Actor::Agent { id: AGENT_PM.into() },
                format!("msg-{}", cause.sequence + 1).as_str(),
                "message",
                EventType::MessageSent,
                serde_json::json!({
                    "to": "owner",
                    "body": "We need one call from you: which database for this build? I recommend A (PostgreSQL) for headroom, but B (SQLite) is zero-infra and cheaper."
                }),
                linked(cause, &corr),
            ),
        ],
    )
    .await?;
    n += 2;

    Ok(n)
}

/// Owner just messaged but we already have requirements — acknowledge politely.
async fn script_acknowledge(state: &AppState, cause: &Event) -> Result<u32> {
    let project = state.project.clone();
    let body = cause
        .data
        .get("body")
        .and_then(|b| b.as_str())
        .unwrap_or("");
    let corr = format!("msg-{}", cause.event_id);
    let msg = ev(
        &project,
        Actor::Agent {
            id: AGENT_PM.into(),
        },
        format!("reply-{}", cause.sequence).as_str(),
        "message",
        EventType::MessageSent,
        serde_json::json!({
            "to": "owner",
            "body": format!("Noted: \u{201c}{body}\u{201d}. It's on the backlog — I'll fold it into the next build pass.")
        }),
        linked(cause, &corr),
    );
    emit(state, vec![msg]).await?;
    Ok(1)
}

/// The owner ruled on a proposed decision — record the verdict's consequences.
async fn script_owner_decision(state: &AppState, cause: &Event) -> Result<u32> {
    let project = state.project.clone();
    let decision_id = cause.aggregate.id.clone();
    let approved = cause
        .data
        .get("approved")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let note = cause
        .data
        .get("note")
        .and_then(|b| b.as_str())
        .unwrap_or("");
    let subject = cause
        .data
        .get("subject")
        .and_then(|b| b.as_str())
        .unwrap_or("your decision");
    let corr = format!("decision-{}", decision_id);

    let mut out = vec![ev(
        &project,
        Actor::Agent {
            id: AGENT_PM.into(),
        },
        format!("msg-{}", cause.sequence).as_str(),
        "message",
        EventType::MessageSent,
        serde_json::json!({
            "to": "owner",
            "body": if approved {
                format!("Great — \u{201c}{subject}\u{201d} approved{}. I'll drive the implementation now.", fmt_note(note))
            } else {
                format!("Understood — \u{201c}{subject}\u{201d} was declined{}. Discarding that option.", fmt_note(note))
            }
        }),
        linked(cause, &corr),
    )];

    if approved {
        out.push(ev(
            &project,
            Actor::Agent { id: AGENT_PM.into() },
            "task-adopt",
            "task",
            EventType::TaskCreated,
            serde_json::json!({"title": format!("Adopt {subject} (owner-approved)"), "kind": "feature"}),
            linked(cause, &corr),
        ));
        out.push(ev(
            &project,
            Actor::Agent {
                id: AGENT_ENG.into(),
            },
            "task-adopt",
            "task",
            EventType::TaskStarted,
            serde_json::json!({}),
            linked(cause, &corr),
        ));
        out.push(ev(
            &project,
            Actor::Agent {
                id: AGENT_ENG.into(),
            },
            "task-adopt",
            "task",
            EventType::TaskCompleted,
            serde_json::json!({"result": format!("Adopted {subject}")}),
            linked(cause, &corr),
        ));
    }

    let count = out.len();
    emit(state, out).await?;
    Ok(count as u32)
}

fn fmt_note(note: &str) -> String {
    if note.is_empty() {
        String::new()
    } else {
        format!(" (\u{201c}{note}\u{201d})")
    }
}
