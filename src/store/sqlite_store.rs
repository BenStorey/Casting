//! SQLite event store, cursor store, and snapshot store.
//!
//! Default zero-dependency deployment option (docs/CASTING_PROJECT_BRIEF.md §29):
//! three DB files with WAL mode for concurrent readers/writers — one for events
//! (`events.db`), one for cursors (`cursors.db`), one for projection snapshots
//! (`snapshots.db`). The events table is append-only; sequence is enforced
//! unique per project for monotonic ordering.

use crate::event::{Actor, Aggregate, Event, EventType, Metadata};
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
/// for slice one; Postgres is a separate backend module.
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
        // Exclusive locking mode prevents a second `cast run` process from opening
        // the same database file, avoiding dual-PM-process conflicts on the .casting/ dir.
        conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
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
        let mut conn = self.conn.lock().unwrap();
        // Next sequence = (current max for project) + 1. Serialized in-process
        // by the mutex; the IMMEDIATE write-lock transaction additionally makes
        // MAX + INSERT atomic against any second writer to the same DB file, so
        // a concurrent process can't observe a sequence gap or race the
        // UNIQUE(project_id, sequence) constraint.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let max: Option<i64> = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE project_id = ?1",
                [&event.project_id],
                |r| r.get(0),
            )
            .optional()?;
        event.sequence = max.unwrap_or(0) + 1;

        let (actor_type, actor_id) = match &event.actor {
            Actor::Director { .. } => ("owner", None),
            Actor::Agent { id } => ("agent", Some(id.clone())),
            Actor::System => ("system", None),
        };

        tx.execute(
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

        tx.commit()?;
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
        // Collect raw rows first, then parse with proper error handling
        // (avoiding unwrap() on corrupt data — §4.1.2).
        type RawEventRow = (
            String,
            String,
            i64,
            String,
            String,
            Option<String>,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let raw: Vec<RawEventRow> = stmt
            .query_map(params![project_id, after], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, Option<String>>(10)?,
                    r.get::<_, Option<String>>(11)?,
                    r.get::<_, Option<String>>(12)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut events = Vec::with_capacity(raw.len());
        for (
            event_id_str,
            project_id,
            sequence,
            timestamp_str,
            actor_type,
            actor_id,
            event_type_str,
            agg_kind,
            agg_id,
            data_str,
            correlation_id,
            causation_id,
            agent_run_id,
        ) in raw
        {
            let event_id = uuid::Uuid::parse_str(&event_id_str)
                .with_context(|| format!("invalid uuid in event_id column: {event_id_str:?}"))?;
            let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                .with_context(|| format!("invalid timestamp column: {timestamp_str:?}"))?
                .with_timezone(&chrono::Utc);
            let actor = match actor_type.as_str() {
                "owner" => Actor::Director {
                    user_id: "ceo".into(),
                },
                "system" => Actor::System,
                _ => Actor::Agent {
                    id: actor_id.unwrap_or_default(),
                },
            };
            let event_type: EventType = serde_json::from_str(&event_type_str)
                .with_context(|| format!("invalid event_type json: {event_type_str:?}"))?;
            let aggregate = Aggregate {
                kind: agg_kind,
                id: agg_id,
            };
            let data: serde_json::Value = serde_json::from_str(&data_str)
                .with_context(|| format!("invalid data json for event {event_id}"))?;
            let causation = causation_id.and_then(|s| uuid::Uuid::parse_str(&s).ok());
            events.push(Event {
                event_id,
                project_id,
                sequence,
                timestamp,
                actor,
                event_type,
                aggregate,
                data,
                metadata: Metadata {
                    correlation_id,
                    causation_id: causation,
                    agent_run_id,
                },
            });
        }
        Ok(events)
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

    fn list_projects(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT DISTINCT project_id FROM events ORDER BY project_id ASC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

// NOTE: `event_type` is stored as its serde string (e.g. "TaskCompleted");
// EventType's serde repr uses the variant name, so round-trips cleanly.
