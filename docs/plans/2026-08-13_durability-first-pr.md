# Durable Execution — First PR (Executor Idempotency)

**Date:** 2026-08-13
**Status:** ✅ **SHIPPED / CLOSED.** First PR landed as commit `825c7d8` (`feat(dur)`: `src/executor.rs` idempotency guard + `redispatch_inflight` boot pass, three `Activity*` events, 7 integration tests, gate green, pushed). **Decision (Ben 2026-08-13): the timer follow-up is deliberately NOT built.** Building `TimerScheduled/Fired/Cancelled` + a derived poller now would be speculative machinery — no deadline-triggered behavior exists yet to fire anything (the PM never blocks on a wall clock; owner decisions are processed on webhook, not by timeout). Revisit ONLY when a real deadline-triggered transition/action appears (auto-timeout, auto-cancel, reminder).

## Why

Casting already survives crashes *without losing state* — the event log is the
only authority, `Projection::build` recomputes state on demand, and the PM loop
resumes from its durable `PM_CONSUMER` cursor on boot. So "disposable process
memory" is not new; it's the existing architecture.

The one place a crash can actually cause harm today is **side-effecting
execution**: an LLM call, `git push`, or shell command that *started* but whose
result never landed because the server died mid-call. State can't be lost, but
physical work can be **re-run** (double LLM spend, a re-push) when the PM drains
from a stale cursor. This PR closes that gap with a tiny, deterministic
idempotency mechanism — **without** adopting a workflow abstraction the
codebase doesn't need.

## Design decisions (from the review — do not re-litigate)

1. **No `workflow_id` + per-workflow `event_number`.** The task `aggregate` +
   the global `sequence` already give deterministic total + per-task ordering.
   A second ordering axis would ripple through every reducer, SSE, provenance,
   and snapshot for zero benefit. A "workflow" == a task aggregate id.
2. **No parallel workflow state machine.** Crash state is already derivable from
   the projection + `graph.rs` (`awaiting_human`, open decision). Dedicated
   `WaitingForActivity/Timer/Human` states would be a second authority over the
   graph spine.
3. **No blocking `recover_workflows` dispatcher.** It re-implements the cursor
   drain. Recovery = a narrow boot pass that re-dispatches in-flight side
   effects, then the normal loop takes over.
4. **Timers are a derived view over the event log, not a second table.**
   Deferred to a follow-up (see Out of scope) — there are no real timers yet.
5. **The executor appends its result and lets the broadcast wake the PM loop.**
   It must NOT call `pm.evaluate` directly, or the executor couples to the PM
   and fights the cursor model.

## The gap

Every discrete side-effecting action gets a **stable activity id** and a
**durable scheduled/completed/failed record** in the event log. Before doing
physical work, the executor checks the log: completed already → skip (return the
cached/idempotent no-op); never completed → execute, then append the result.
After a crash, a boot pass re-dispatches anything that is scheduled-but-incomplete.

## Event model additions (the only schema change)

Three new curated `EventType` variants (extend deliberately):

- `ActivityScheduled` — `{ activity_id, kind, target_id }`. Records *intent* to
  run a side effect. `activity_id` is stable and derived, e.g.
  `"{target_id}-{n}"` (LLM call #3 on task-7 → `task-7-llm-call-3`).
- `ActivityCompleted` — `{ activity_id, result_ref }`. The durable answer; this
  is the idempotency marker. `result_ref` points to stored artifact (see
  Open questions), never an inline megapayload.
- `ActivityFailed` — `{ activity_id, error }`. Feeds a retry *decision* (PM
  layer), never a machinery-retry counter.

All three fold deterministically — the projection can answer
`is_activity_completed(task_id, activity_id)` (or a helper on `AppState`).

## The executor contract (`src/executor.rs`, new)

```text
fn execute(state, target_id, activity) -> Result<()>:
    if is_activity_completed(target_id, activity.id): return Ok(())   # idempotent
    result = match activity.kind:
        LlmCall  => openrouter.complete(prompt).await     # D2 seam
        GitPush  => git.push(branch).await
        Shell    => run_shell(cmd).await
        _        => inline/derived, no external work
    append(ActivityCompleted { activity.id, result_ref })  # broadcast wakes PM loop
```

- First call sites: the **D2 orchestrator seam** (it's unplugged today — the
  guard is written now so the real LLM lands crash-safe when wired) and the
  **git observer / worktree prune** (only real side effects running today).
- The sync `AppState::append` already broadcasts — the PM loop picks up the
  `ActivityCompleted` and drains, so no direct `pm.evaluate` call.

## Boot re-dispatch pass (small, non-blocking)

In `main.rs` before the PM loop starts (a function, spawned as a task):

1. Read all `ActivityScheduled` events; for each without a matching
   `ActivityCompleted`/`ActivityFailed`, call `execute(...)` (idempotency guard
   makes a re-run safe).
2. Let the normal PM loop/cursor take over from there.

No state walk, no per-branch match on workflow states — just
"scheduled-but-incomplete → re-dispatch."

## Open questions — answered

- **Append-only monotonic?** Yes, global `sequence` per project. No
  per-workflow `event_number` (Decision 1).
- **Timers: table or view?** Derived view of the event log (Decision 4) —
  follow-up PR, not this one.
- **Large activity results?** **Reference** (`result_ref` to a stored artifact),
  never inline — same precedent as briefing/diagram assets (they store a path,
  not the payload).
- **max_retries?** **PM/decision layer**, not executor. `ActivityFailed` feeds a
  retry decision; the machinery records, the decision decides.
- **Recovery loop: task vs block startup?** Non-blocking boot pass (above),
  not a separate state-walking dispatcher.

## Out of scope (follow-ups, not this PR)

- Timer persistence (`TimerScheduled/Fired/Cancelled` events + derived
  poller) — build once there is a real blocking wait to persist; covered in the
  full durability plan, deliberately deferred here.
- Wiring a real LLM into the executor (D2 — still deferred; the guard is the
  prep).
- `workflow_id`/workflow-event dimension (Decision 1 — never).

## Test plan (TDD)

1. Idempotency: schedule → complete → re-schedule/execute is a no-op (no second
   side effect; the mock executor counts invocations).
2. Crash mid-activity: activity scheduled but not completed → boot pass
   re-dispatches exactly once.
3. `is_activity_completed` folds correctly from raw events; ordering by
   `sequence` is deterministic.
4. Executor appending `ActivityCompleted` wakes the PM drain (no direct
   `pm.evaluate` coupling).
5. Scripted suite stays green (these events are additive; no existing reducer
   changes).

## Gate

`make` (fmt → clippy `-D warnings` → test → build) green; commit + push per
standing instruction.
