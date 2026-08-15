//! Durable per-consumer cursors into the event history.
//!
//! Every consumer (an agent, the PM, a projection) keeps a position
//! (docs/CASTING_PROJECT_BRIEF.md §16). It can resume from its last seen
//! sequence after a crash/restart. A notification is a hint to consume
//! persisted events — the cursor is the durable position, not transient
//! messaging (docs/CASTING_PROJECT_BRIEF.md §17).

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Durable position of one consumer within one project's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub project_id: String,
    /// Stable consumer id, e.g. "pm", "agent:marcus-reed", "projection:board".
    pub consumer: String,
    /// Last sequence this consumer has processed.
    pub last_seen: i64,
}

/// The cursor-storage abstraction. Concrete backends (SQLite, Postgres)
/// implement this; callers never touch a concrete type directly (owner
/// principle: every store read/write goes through the abstraction).
pub trait CursorStore: Send + Sync {
    /// Current position for a consumer, or `last_seen = 0` if never seen.
    fn get(&self, project_id: &str, consumer: &str) -> Result<Cursor>;

    /// Advance a consumer to the given sequence (idempotent update).
    fn advance(&self, project_id: &str, consumer: &str, to: i64) -> Result<()>;
}

/// Allow `Arc<dyn CursorStore>` (what `AppState` holds) to act as a
/// `CursorStore` — the backend is behind the trait.
impl CursorStore for std::sync::Arc<dyn CursorStore> {
    fn get(&self, project_id: &str, consumer: &str) -> Result<Cursor> {
        (**self).get(project_id, consumer)
    }
    fn advance(&self, project_id: &str, consumer: &str, to: i64) -> Result<()> {
        (**self).advance(project_id, consumer, to)
    }
}

const CURSOR_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS cursors (
    project_id  TEXT NOT NULL,
    consumer    TEXT NOT NULL,
    last_seen   INTEGER NOT NULL,
    PRIMARY KEY (project_id, consumer)
);
"#;

/// SQLite-backed cursor storage. Designed to be opened on the same DB file as
/// the event store so cursors and history live together (a project's whole
/// state can be copied/backed up as one file — docs/CASTING_PROJECT_BRIEF.md §29).
#[derive(Clone)]
pub struct SqliteCursorStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteCursorStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(CURSOR_SCHEMA)?;
        Ok(SqliteCursorStore {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(CURSOR_SCHEMA)?;
        Ok(SqliteCursorStore {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl crate::store::CursorStore for SqliteCursorStore {
    fn get(&self, project_id: &str, consumer: &str) -> Result<Cursor> {
        let conn = self.conn.lock().unwrap();
        let last_seen: Option<i64> = conn
            .query_row(
                "SELECT last_seen FROM cursors WHERE project_id = ?1 AND consumer = ?2",
                params![project_id, consumer],
                |r| r.get(0),
            )
            .optional()?;
        Ok(Cursor {
            project_id: project_id.to_string(),
            consumer: consumer.to_string(),
            last_seen: last_seen.unwrap_or(0),
        })
    }

    fn advance(&self, project_id: &str, consumer: &str, to: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cursors (project_id, consumer, last_seen)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (project_id, consumer)
             DO UPDATE SET last_seen = excluded.last_seen",
            params![project_id, consumer, to],
        )?;
        Ok(())
    }
}
