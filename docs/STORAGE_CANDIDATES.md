# Storage candidates & trade-offs (immutable rationale)

> Why we chose SQLite + Postgres, and what else would fit the design if we ever
> wanted more choices. This is immutable architectural rationale (the one kind
> of markdown Casting allows) — the reasoning behind decisions, not runnable
> state. Recorded 2026-08-10 per the owner's "save interesting facts so we
> don't re-derive them" practice.

## What the storage layer actually needs

1. **Event log (source of truth):** append-only, ordered per-project, with a
   monotonic per-project **sequence** assigned atomically with the insert.
   Under write-time integrity this append is **transactional** (a `DecisionMade`
   can't land without its `DecisionProposed`).
2. **Snapshots:** a disposable `project_id → {sequence, projection blob}`
   key-value map, rebuildable from the log — can live anywhere.
3. **Cursors:** tiny `(project, consumer) → int` rows, upserted often.

The binding constraint is **#1**: a store able to do an atomic
"compute next sequence + insert" per project. Backends live behind the
`EventStore` / `CursorStore` / `SnapshotStore` traits, so adding any backend is
a new impl, never a refactor.

## Current choices (default + hosted)

- **SQLite** — the default. Embedded, zero-ops, collocated per-project in
  `<repo>/.casting/` (events.db / cursors.db / snapshots.db). Perfect for the
  single-owner local model.
- **Postgres** — the hosted/swappable backend (real concurrency + durability),
  driven on a dedicated thread so the sync traits work inside our tokio server.
  Selected per run via `cast run <dir> --db <selector>` or
  `CAST_DB`.

## Other databases that fit

Ranked by pragmatic fit:

- **MySQL / MariaDB** — near drop-in (transactional SQL, auto-increment per
  project row). Cheapest third option; only dialect differences.
- **CockroachDB / TiDB** — Postgres/MySQL-compatible distributed DBs. Slot in
  behind the same traits with just a connection string; the "scale out" story.
- **FoundationDB** — ordered key-value with strong transactions. Maps nearly
  1:1 onto our needs: events → `(project, seq)` ordered keys; snapshots →
  `(project, "snap")` blob; cursors → `(project, "cursor", consumer)`. A single
  transaction reads max seq + appends, satisfying the integrity precondition
  atomically. Arguably the most elegant architectural fit.
- **DynamoDB** — partition key `project`, sort key `seq` for the log; per-partition
  ordering + conditional writes (`attribute_not_exists` on seq) give atomic
  append. Caveats: strong-transaction item-count limits (~25/txn) and you'd
  manage TTL for snapshot cleanup. The "serverless hosted" option.

## Partial fits (don't reach)

- **MongoDB** — transactions exist (4.0+), but default ordering isn't
  trustworthy; you'd lean on an explicit seq field + unique index. Least natural
  of the transactional options for strict sequence invariants.
- **Kafka / Redpanda** — a phenomenal event *log* (ordered, partitioned,
  per-partition ordering = exactly our per-project sequence), but it's a log,
  not a queryable DB: you'd still need a separate read model for projections
  and cursors. Layered *with* a store, not instead of one.

## Conclusion

SQLite + Postgres is the right set for real use. If we ever expand: **MySQL**
(pragmatic), **Cockroach/TiDB** (scale-out headroom), **FoundationDB** (most
elegant), **DynamoDB** (serverless). Skip MongoDB and Kafka as store
replacements.