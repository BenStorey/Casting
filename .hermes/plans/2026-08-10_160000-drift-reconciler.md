# Drift Reconciler — cursor-gated "every N events" pass — Implementation Plan

> **For Hermes:** TDD; commit per task; push. Mirrors the PM loop (own cursor,
> own pass). Owner framing (2026-08-10): opinions don't go stale in a burst,
> they drift — reconcile periodically, never at write time.

## Why
Knowledge (opinions) accumulates and drifts. Write-time supersession is eager
and brittle (writer must know every target). Instead: **simple writes, periodic
reconciliation.** A reconciler consumer wakes every N events, detects drift
mechanically, emits ordinary gate actions, advances its own cursor.

## Design

Reusable primitive (also valid later for priorities/plan ranking): **a
cursor-gated reconciliation pass** — `RECONCILER_CONSUMER` cursor + a threshold
interval; fires when `latest - reconciler_cursor >= N`.

Requirement it exposes: opinions need a **`subject`** matching key (from the
earlier design fork, Model-2 mechanics). Without it the mechanical
same-subject contradiction can't be detected.

### 1. Opinion.subject
- Add `subject: String` to `Opinion` (serde default ""), `RecordOpinion` action
  gains `subject`.
- Reducer stores it. Empty subject = ungroupable (reconciler skips).

### 2. Reconciler (src/reconciler.rs)
- Const `RECONCILER_CONSUMER = "reconciler"`.
- AppState field `reconcile_interval: u64` (default 25) + builder
  `with_reconcile_interval`, so tests set it low.
- `drift(projection) -> Vec<Reconcile>`: pairs of Active opinions with the SAME
  subject (ordinal = chronological; the earlier is superseded by the later).
- `should_run(state)`: due when `latest - reconciler_cursor >= interval`.
- `reconcile(state) async`: build projection, detect drift, for each emit
  `SupersedeOpinion` (older by newer) through the gate (reusing run_planned
  logic path), advance reconciler cursor to latest.
- `drive_reconciler(state)` = one-pass entry for tests/CLI.

### 3. Wire into PM loop
- `run_pm`: after `drain`, if `should_run` then `reconcile`. Also after each
  `drive_pm` in tests we call `drive_reconciler`.
- Gate rejects any SupersedeOpinion that no longer holds (already-flipped) —
  intermediate projection keeps reconcile idempotent.

## Tasks
1. `subject` on Opinion + RecordOpinion + reducer (back-compat serde default).
2. Reconciler: cursor + interval + drift detection + reconcile pass.
3. Wire into PM loop + tests (fake drift: two active same-subject opinions ->
   reconcile supersedes the older; unrelated/empty-subject untouched; gate
   guard).
4. Docs + full gate + push.