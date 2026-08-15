//! Persistence backends — event store, cursors, snapshots.
//!
//! All storage is behind traits so the binary can swap between SQLite (default /
//! self-contained) and PostgreSQL (multi-project / production) without changing
//! any consuming code.

pub mod backend;
pub mod cursor;
pub mod event_store;
pub mod postgres_store;
pub mod snapshot;
pub mod sqlite_store;

pub use backend::{from_selector, Backend};
pub use cursor::{Cursor, CursorStore, SqliteCursorStore};
pub use event_store::EventStore;
pub use postgres_store::PostgresBackend;
pub use snapshot::{build_from, SnapshotStore, SqliteSnapshotStore};
pub use sqlite_store::SqliteEventStore;
