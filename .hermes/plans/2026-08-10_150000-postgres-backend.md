# Storage abstraction + Postgres backend — Implementation Plan

> **For Hermes:** implement with TDD; commit after each task; push. This is the
> storage seam getting real. Owner principle (2026-08-10): **every store read/
> write goes through the abstraction layer** — SQLite is ONE backend behind it,
> Postgres is another, freely swappable. We do NOT carry both as parallel
> concrete paths in AppState; AppState talks to traits.

## Design

Three traits, one chosen backend behind them:

- `EventStore` (exists: append/read_since/latest_sequence/list_projects)
- `CursorStore` (new trait: get/advance)
- `SnapshotStore` (new trait: save/load/clear)

Concrete impls:
- **SQLite backend** (rename, keep behavior): `SqliteEventStore` (exists),
  `SqliteCursorStore` (was `CursorStore`), `SqliteSnapshotStore` (was
  `SnapshotStore`).
- **Postgres backend** (new): `PostgresEventStore`, `PostgresCursorStore`,
  `PostgresSnapshotStore` — all via the synchronous `postgres` crate so the
  sync traits stay sync.

`AppState` holds trait objects:
- `store: Arc<dyn EventStore>`
- `cursors: Arc<dyn CursorStore>`
- `snapshots: Option<Arc<dyn SnapshotStore>>`

A single `Storage` factory (or `Backend` enum) constructs all three for a given
config, so a deployment picks one backend (`Sqlite(path)` or
`Postgres(url)`) with no second copy.

## Tasks (commit each)

1. **Traits + SQLite renames.** Add `CursorStore`/`SnapshotStore` traits;
   rename concrete `CursorStore`→`SqliteCursorStore`, `SnapshotStore`→
   `SqliteSnapshotStore`; `AppState` fields become `Arc<dyn ...>`. Fix all
   construction sites (~35 SqliteEventStore, ~29 CursorStore, ~6 SnapshotStore,
   ~23 AppState::new). Gate stays green at same test count.
2. **Postgres impls.** `src/postgres_store.rs` (all three) via `postgres`
   crate; schema mirroring SQLite (events/cursors/projections tables). In-memory
   not needed; a real PG connection per tests.
3. **Backend factory + runtime selection.** `Storage`/`Backend` that builds the
   three from a config string; wire into main.rs `cast run` via the project
   registry entry (e.g. `db: "sqlite" | "postgres://url"`). Add
   `docker-compose.postgres.yml` (or reuse our docker) so the test harness can
   stand up a real Postgres.
4. **Integration test against real Postgres.** Stand up a PG container, run the
   full store round-trip (append/read/latest/list + cursor + snapshot), and a
   company boot end-to-end on Postgres. Docs (store.rs header, HANDOFF roadmap,
   DEPLOYMENT) + push.

## Validation
- `make` full gate after each task.
- Real Postgres integration test (docker) — not a mock.
