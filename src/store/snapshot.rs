//! Projection snapshots — a pure optimization, never a source of truth.
//!
//! Per docs/SEMANTIC_EVENTS.md §18–19, the event log is the only authority; a
//! projection is derived state. Rebuilding the projection from the full log on
//! every request gets expensive, so we periodically persist a serialized
//! projection plus the sequence it was folded up to. `Projection::build_from`
//! loads the snapshot then applies only the events after it. If the snapshot is
//! missing, stale, or fails to deserialize, we fall back to a full fold and
//! discard the bad snapshot — a snapshot can always be thrown away and
//! reconstructed from the log. It can never become a second source of truth.

use crate::projection::Projection;
use anyhow::Result;

/// The projection-snapshot abstraction (pure optimization, never authoritative).
/// Concrete backends (SQLite, Postgres) implement this; callers never touch a
/// concrete type directly (owner principle: every store read/write goes
/// through the abstraction).
pub trait SnapshotStore: Send + Sync {
    /// Persist a snapshot of the projection folded up to `sequence`.
    fn save(&self, project_id: &str, sequence: i64, projection: &Projection) -> Result<()>;

    /// Load the latest snapshot: `(sequence, projection)` or `None` if absent /
    /// unreadable. A deserialization failure returns `None` so the caller falls
    /// back to a full fold (snapshots are disposable).
    fn load(&self, project_id: &str) -> Option<(i64, Projection)>;

    /// Drop this project's snapshot (e.g. on a full replay). Never required.
    fn clear(&self, project_id: &str) -> Result<()>;
}

/// Allow `Arc<dyn SnapshotStore>` (what `AppState` holds) to act as a
/// `SnapshotStore` — the backend is behind the trait.
impl SnapshotStore for std::sync::Arc<dyn SnapshotStore> {
    fn save(&self, project_id: &str, sequence: i64, projection: &Projection) -> Result<()> {
        (**self).save(project_id, sequence, projection)
    }
    fn load(&self, project_id: &str) -> Option<(i64, Projection)> {
        (**self).load(project_id)
    }
    fn clear(&self, project_id: &str) -> Result<()> {
        (**self).clear(project_id)
    }
}

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

const SNAPSHOT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projections (
    project_id   TEXT NOT NULL PRIMARY KEY,
    sequence     INTEGER NOT NULL,
    projection   TEXT NOT NULL
);
"#;

/// SQLite-backed snapshot store. Opened on the same DB file as the event store
/// so a project's state is one file (brief §29), mirroring the cursor store.
#[derive(Clone)]
pub struct SqliteSnapshotStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteSnapshotStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SNAPSHOT_SCHEMA)?;
        Ok(SqliteSnapshotStore {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SNAPSHOT_SCHEMA)?;
        Ok(SqliteSnapshotStore {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl crate::store::SnapshotStore for SqliteSnapshotStore {
    fn save(&self, project_id: &str, sequence: i64, projection: &Projection) -> Result<()> {
        let json = serde_json::to_string(projection)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO projections (project_id, sequence, projection)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (project_id) DO UPDATE SET sequence = excluded.sequence,
                                                    projection = excluded.projection",
            params![project_id, sequence, json],
        )?;
        Ok(())
    }

    fn load(&self, project_id: &str) -> Option<(i64, Projection)> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(i64, String)> = conn
            .query_row(
                "SELECT sequence, projection FROM projections WHERE project_id = ?1",
                params![project_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()?;
        let (seq, json) = row?;
        serde_json::from_str(&json).ok().map(|p| (seq, p))
    }

    fn clear(&self, project_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM projections WHERE project_id = ?1",
            params![project_id],
        )?;
        Ok(())
    }
}

/// Build a projection from a snapshot when one exists, folding only the tail
/// events after it; otherwise full-fold. Thus a snapshot changes nothing about
/// the *result* — only how it was computed. If the snapshot is unusable we fall
/// back to `Projection::build`. This helper does NOT write new snapshots (writes
/// are the caller's choice), so the read path can stay snapshot-agnostic.
pub fn build_from<S: crate::store::EventStore>(
    store: &S,
    snapshots: &dyn SnapshotStore,
    project_id: &str,
) -> Result<Projection> {
    match snapshots.load(project_id) {
        Some((seq, mut proj)) => {
            let tail = store.read_since(project_id, seq)?;
            for e in &tail {
                proj.apply(e);
            }
            Ok(proj)
        }
        None => Projection::build(store, project_id),
    }
}
