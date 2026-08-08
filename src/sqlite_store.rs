//! SQLite append-only event store.
//!
//! Default zero-dependency deployment option (docs/CASTING_PROJECT_BRIEF.md §29):
//! a single DB file, WAL mode for concurrent readers/writers. Events table is
//! append-only; sequence is enforced unique per project for monotonic ordering.

use crate::event::{Actor, Aggregate, Event, Metadata};
use crate::store::EventStore;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The single append-only events table.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS events (
    event_id      TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL,
    sequence      INTEGER NOT NULL,
    timestamp     TEXT NOT NULL,
    actor_type    TEXT NOT NULL,
    actor_id      TEXT,
    event_type    TEXT NOT NULL,
    aggregate_kind TEXT NOT NULL,
    aggregate_id  TEXT NOT NULL,
    data          TEXT NOT NULL,
    correlation_id TEXT,
    causation_id  TEXT,
    agent_run_id  TEXT,
    UNIQUE (project_id, sequence)
);
CREATE INDEX IF NOT EXISTS idx_events_project_sequence
    ON events (project_id, sequence);
"#;

/// SQLite-backed [`EventStore`]. Cheap interior mutability via a Mutex is fine
/// for slice one; Postgres (real concurrency) is a later backend.
#[derive(Clone)]
pub struct SqliteEventStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteEventStore {
    /// Open (creating if needed) a store at `path`, applying schema + WAL mode.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create parent dir for {}", path.display()))?;
            }
        }
        let conn = Connection::open(path)
            .with_context(|| format!("open SQLite database {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(SqliteEventStore {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory store (tests / ephemeral runs).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(SqliteEventStore {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl EventStore for SqliteEventStore {
    fn append(&self, mut event: Event) -> Result<Event> {
        let conn = self.conn.lock().unwrap();
        // Next sequence = (current max for project) + 1. Serialized by the
        // mutex, so this is atomic for slice one.
        let max: Option<i64> = conn
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE project_id = ?1",
                [&event.project_id],
                |r| r.get(0),
            )
            .optional()?;
        event.sequence = max.unwrap_or(0) + 1;

        let (actor_type, actor_id) = match &event.actor {
            Actor::Owner => ("owner", None),
            Actor::Agent { id } => ("agent", Some(id.clone())),
            Actor::System => ("system", None),
        };

        conn.execute(
            "INSERT INTO events (
                event_id, project_id, sequence, timestamp,
                actor_type, actor_id, event_type,
                aggregate_kind, aggregate_id, data,
                correlation_id, causation_id, agent_run_id
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                event.event_id.to_string(),
                event.project_id,
                event.sequence,
                event.timestamp.to_rfc3339(),
                actor_type,
                actor_id,
                serde_json::to_string(&event.event_type)?,
                event.aggregate.kind,
                event.aggregate.id,
                serde_json::to_string(&event.data)?,
                event.metadata.correlation_id,
                event.metadata.causation_id.map(|id| id.to_string()),
                event.metadata.agent_run_id,
            ],
        )
        .with_context(|| format!("insert event seq {}", event.sequence))?;

        Ok(event)
    }

    fn read_since(&self, project_id: &str, after: i64) -> Result<Vec<Event>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT event_id, project_id, sequence, timestamp,
                    actor_type, actor_id, event_type,
                    aggregate_kind, aggregate_id, data,
                    correlation_id, causation_id, agent_run_id
             FROM events
             WHERE project_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC",
        )?;
        let rows = stmt.query_map(params![project_id, after], |r| {
            let actor_type: String = r.get(4)?;
            let actor_id: Option<String> = r.get(5)?;
            let actor = match actor_type.as_str() {
                "owner" => Actor::Owner,
                "system" => Actor::System,
                _ => Actor::Agent {
                    id: actor_id.unwrap_or_default(),
                },
            };
            let event_type: String = r.get(6)?;
            let correlation_id: Option<String> = r.get(10)?;
            let causation_id: Option<String> = r.get(11)?;
            let agent_run_id: Option<String> = r.get(12)?;
            Ok(Event {
                event_id: uuid::Uuid::parse_str(&r.get::<_, String>(0)?).unwrap(),
                project_id: r.get(1)?,
                sequence: r.get(2)?,
                timestamp: chrono::DateTime::parse_from_rfc3339(&r.get::<_, String>(3)?)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                actor,
                event_type: serde_json::from_str(&event_type).unwrap(),
                aggregate: Aggregate {
                    kind: r.get(7)?,
                    id: r.get(8)?,
                },
                data: serde_json::from_str(&r.get::<_, String>(9)?).unwrap(),
                metadata: Metadata {
                    correlation_id,
                    causation_id: causation_id.and_then(|s| uuid::Uuid::parse_str(&s).ok()),
                    agent_run_id,
                },
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn latest_sequence(&self, project_id: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let max: Option<i64> = conn
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE project_id = ?1",
                [project_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(max.unwrap_or(0))
    }
}

// NOTE: `event_type` is stored as its serde string (e.g. "TaskCompleted");
// EventType's serde repr uses the variant name, so round-trips cleanly.
