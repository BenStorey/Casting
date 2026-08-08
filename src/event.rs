//! Typed domain events — the append-only history is the source of truth.
//!
//! Mirrors the event anatomy in docs/CASTING_PROJECT_BRIEF.md §11 and the
//! domain-vs-runtime separation in §12. Only DOMAIN events belong here;
//! low-level telemetry (token streams, shell commands, git plumbing) lives
//! elsewhere and never enters this store's history.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Who or what caused an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    /// The human owner.
    Owner,
    /// An agent, identified by its stable id (e.g. "marcus-reed", "pm").
    Agent { id: String },
    /// The system itself (e.g. background watchers, persistence).
    System,
}

/// Semantic type of a domain event. Deliberately a small, curated set —
/// the organizational model (docs/CASTING_PROJECT_BRIEF.md §12) rather
/// than raw machinery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    ProjectCreated,
    AgentHired,
    RequirementCreated,
    RequirementChanged,
    TaskCreated,
    TaskAssigned,
    TaskStarted,
    TaskCompleted,
    TaskBlocked,
    ObservationCreated,
    DecisionProposed,
    OwnerDecisionRecorded,
    MessageSent,
}

/// The entity primarily affected by an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aggregate {
    pub kind: String,
    pub id: String,
}

/// Correlation/causation links (docs/CASTING_PROJECT_BRIEF.md §11).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// The larger operation this event belongs to.
    pub correlation_id: Option<String>,
    /// The event that directly caused this one.
    pub causation_id: Option<Uuid>,
    /// The underlying agent/model execution, if any.
    pub agent_run_id: Option<String>,
}

/// One immutable, append-only domain event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Globally unique id.
    pub event_id: Uuid,
    pub project_id: String,
    /// Monotonically increasing sequence within the project. Authoritative
    /// ordering — do not rely on timestamps alone.
    pub sequence: i64,
    pub timestamp: DateTime<Utc>,
    pub actor: Actor,
    pub event_type: EventType,
    pub aggregate: Aggregate,
    /// Event-specific structured payload (JSON-compatible).
    pub data: serde_json::Value,
    pub metadata: Metadata,
}

impl Event {
    /// Build a new event with a fresh id/timestamp and the supplied fields.
    /// `sequence` is supplied by the store on append, not here.
    pub fn new(
        project_id: impl Into<String>,
        actor: Actor,
        event_type: EventType,
        aggregate: Aggregate,
        data: serde_json::Value,
    ) -> Self {
        Event {
            event_id: Uuid::new_v4(),
            project_id: project_id.into(),
            sequence: 0, // assigned on append
            timestamp: Utc::now(),
            actor,
            event_type,
            aggregate,
            data,
            metadata: Metadata::default(),
        }
    }
}
