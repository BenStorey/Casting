# Casting — Holistic Review Remediation (2026-08-14)

Status: **COMPLETE — all 18 review findings addressed across 16 batches, pushed to
`main` (052000c..0e63661). Full `make` gate green, 342 tests passing (was ~217).**
Owner: Ben · Author: Hermes
Method: fixed in testable, committable batches; `make` green + commit + push per batch.

Batches 1–16 all shipped. New modules/tests added: `src/planning.rs` (PM planning
extracted from the pm.rs monolith), `tests/roundtrip.rs`, `tests/drain_idempotency.rs`,
`tests/transition_consistency.rs`, `tests/api_contract.rs`, `frontend/src/boardColumns.ts`.
Deliberately deferred (documented, safe to pick up later): deep `AppState` field-layout
slimming (B16 did conservative code-motion only) and the fuller frontend codegen for
`api.ts` (B15 shipped the deterministic Rust-side contract pin instead; a codegen step
remains an option).

## Review summary (what we're fixing)

The holistic review (3 deep readers + Hermes verification) found ~18 items across
**backends correctness, structural drift, and frontend**. Big idea: the event-log
single-authority spine is solid; the problems are (a) a handful of real correctness/
robustness bugs, (b) drift — the hand-maintained `api.ts` mirror undercuts "one
authority", the task state-machine is encoded twice, telemetry rides the domain log, and
(c) monolith growth in `pm.rs`. Honestly: several are pre-existing warts the feature
rush left behind.

**Three headline bugs verified by Hermes directly:**
1. Board drops any task in `InReview` (TS `TaskStatus` lacks `in_review`; `App.tsx` 4-col filter).
2. Observers (`git_observer`/`reconciler`/watchdog) append raw to the store, bypassing `integrity::check_append`.
3. `run_planned` runs the physical side effect BEFORE appending the intent event, so a failed
   physical op still gets recorded (projection can lie about worktrees).

## Batches (each = one or more commits + push)

| # | Batch | Items | Risk |
|---|-------|-------|------|
| 1 | Frontend drive-by: Board InReview, Activity failure highlight, frontend CI (typecheck/lint/test), pkg/css hygiene | #1 #18 (+extras) | low |
| 2 | Round-trip + idempotency regression test (all PmActions: validate→to_events→apply; fold-twice identity) | #12 | low |
| 3 | Side-effect ordering + explicit failure marker | #3 | med |
| 4 | Unified guarded append path (integrity for all writers) | #2 | med |
| 5 | PM drain transactional/idempotent (events+cursor commit together) | #6 | med |
| 6 | `GET /api/health` + atomic sequence allocation | #14 #15 | med |
| 7 | Snapshot save off the read path (on cursor-advance / cadence) | #16 | low |
| 8 | Postgres reconnect+backoff, reply timeout, don't block tokio workers | #4 | high |
| 9 | Auth: guard `/api/setup` + `/api/telegram/configure`; refuse silent token rotation | #5 | med |
| 10 | Telegram outbound dedup (advance out-cursor even on send failure) | #7 | low |
| 11 | Watchdog retry-windowing + surface drain/store failures as events | #17 | med |
| 12 | Single task state machine (graph TABLE drives the gate) | #9 | high |
| 13 | Merge PolicyError hierarchies | #10 | med |
| 14 | Telemetry vs domain-log decision (doc+code consistent) | #11 | low |
| 15 | api.ts frontend mirror: codegen (ts-rs/schemars) or contract test | #8 | high |
| 16 | pm.rs split + slim AppState | #13 | high |

## Seam notes / conventions that hold throughout
- All appends go through ONE guarded path once batch 4 lands; observe-before-append is fixed in batch 3.
- `make` stays the one-step gate (fmt→clippy -D warnings→test→build); commit + push after each batch.
- Keep `EventType` a curated enum; extend deliberately.
- Round-trip test (batch 2) becomes the regression net that any later event/action/state change must pass.
- Frontend: any TS mirror change lands with its Rust source in the SAME commit so the two can't drift further.
