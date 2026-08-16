//! Event-store abstraction.
//!
//! The logical event model is database-independent (docs/CASTING_PROJECT_BRIEF.md
//! §10). SQLite and Postgres backends are both implemented behind the same trait. Do not force a lowest-common-denominator
//! SQL layer — the trait only needs what the domain requires.

use crate::event::Event;
use anyhow::Result;
use std::sync::Arc;

/// Append-only event persistence.
pub trait EventStore: Send + Sync {
    /// Append an event, assigning it the next monotonic sequence for its
    /// project. Returns the event with `sequence` populated.
    fn append(&self, event: Event) -> Result<Event>;

    /// Read all events for a project with `sequence > after`.
    /// Ordering is by ascending sequence (deterministic, append order).
    fn read_since(&self, project_id: &str, after: i64) -> Result<Vec<Event>>;

    /// Highest sequence currently stored for a project (or 0 if empty).
    fn latest_sequence(&self, project_id: &str) -> Result<i64>;

    /// All distinct project ids present in the store, ascending.
    fn list_projects(&self) -> Result<Vec<String>>;
}

/// Allow `Arc<dyn EventStore>` (what `AppState` holds) to be used everywhere a
/// `EventStore` is expected — the runtime is behind the trait, callers don't
/// care whether it's SQLite or Postgres.
impl EventStore for Arc<dyn EventStore> {
    fn append(&self, event: Event) -> Result<Event> {
        (**self).append(event)
    }
    fn read_since(&self, project_id: &str, after: i64) -> Result<Vec<Event>> {
        (**self).read_since(project_id, after)
    }
    fn latest_sequence(&self, project_id: &str) -> Result<i64> {
        (**self).latest_sequence(project_id)
    }
    fn list_projects(&self) -> Result<Vec<String>> {
        (**self).list_projects()
    }
}
