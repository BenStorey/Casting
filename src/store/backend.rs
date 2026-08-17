//! Storage backend selection — the runtime behind the storage traits.
//!
//! One deployment picks ONE backend (SQLite or Postgres); `AppState` only ever
//! sees trait objects, so swapping is a one-line config change. This is the
//! director's "freely swap one for the other" seam made concrete.

use crate::store::CursorStore;
use crate::store::EventStore;
use crate::store::SnapshotStore;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

/// A concrete storage backend: one handle serving all three traits.
pub enum Backend {
    /// Default: a per-project SQLite DB file under the project's `~/.casting/<slug>/`.
    Sqlite {
        events: Arc<crate::store::SqliteEventStore>,
        cursors: Arc<crate::store::SqliteCursorStore>,
        snapshots: Arc<crate::store::SqliteSnapshotStore>,
    },
    /// Hosted: everything on a Postgres server (real concurrency + durability).
    Postgres {
        pg: Arc<crate::store::PostgresBackend>,
    },
}

impl Backend {
    /// Open the default SQLite backend for a project's `~/.casting/<slug>/` dir.
    pub fn sqlite(casting_dir: &Path) -> Result<Self> {
        Ok(Backend::Sqlite {
            events: Arc::new(crate::store::SqliteEventStore::open(
                casting_dir.join("events.db"),
            )?),
            cursors: Arc::new(crate::store::SqliteCursorStore::open(
                casting_dir.join("cursors.db"),
            )?),
            snapshots: Arc::new(crate::store::SqliteSnapshotStore::open(
                casting_dir.join("snapshots.db"),
            )?),
        })
    }

    /// Open a Postgres backend from a libpq connection string.
    pub fn postgres(cfg: &str) -> Result<Self> {
        Ok(Backend::Postgres {
            pg: Arc::new(crate::store::PostgresBackend::connect(cfg)?),
        })
    }

    /// The event store (as a trait object for `AppState`/`Projection::build`).
    pub fn events(&self) -> Arc<dyn EventStore> {
        match self {
            Backend::Sqlite { events, .. } => events.clone(),
            Backend::Postgres { pg } => pg.clone(),
        }
    }

    /// The cursor store.
    pub fn cursors(&self) -> Arc<dyn CursorStore> {
        match self {
            Backend::Sqlite { cursors, .. } => cursors.clone(),
            Backend::Postgres { pg } => pg.clone(),
        }
    }

    /// The snapshot store (Snapshots are optional; providing one is standard).
    pub fn snapshots(&self) -> Option<Arc<dyn SnapshotStore>> {
        match self {
            Backend::Sqlite { snapshots, .. } => Some(snapshots.clone()),
            Backend::Postgres { pg } => Some(pg.clone()),
        }
    }
}

/// Parse a `--db` / `CAST_DB` backend selector into a `Backend`.
///   - `sqlite` (default) — per-project `~/.casting/<slug>/` SQLite files.
///   - `postgres://...` / a libpq string — hosted Postgres.
pub fn from_selector(selector: &str, casting_dir: &Path) -> Result<Backend> {
    let sel = selector.trim();
    if sel.is_empty() || sel == "sqlite" {
        Backend::sqlite(casting_dir)
    } else {
        Backend::postgres(sel)
    }
}
