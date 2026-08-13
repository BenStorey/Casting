# Harness guards — budget breaker, secrets seam, liveness watchdog

> Owner decision 2026-08-13 (while traveling): D2 (real LLM) wires up in ~2 days.
> Build the deterministic, LLM-free guard rails THAT ARE VALUABLE AFTERWARDS now,
> and hold the ones that are speculative. Self-actuating guards first; owner
> notification is deferred (messaging wiring is deferred until Ben is back).

## The one-line stance (extends docs/HARNESS.md)

> The PM *optimizes*; the guard *refuses*. The hard rails live OUTSIDE the PM's
> control — in a pure projection + the dispatch gate — because the PM is an agent
> that can be confused, compromised, or stuck. Event-sourced, deterministic,
> gate-enforced: same grain as everything else. LLM-free, so buildable now.

## Sequencing (what we build, what we hold)

| # | Guard | Build now? | Why |
|---|-------|-----------|-----|
| 1 | **Shared halt/pause + budget breaker** | ✅ Yes | Real money moves once D2 wires; owner is traveling/unattended. Attribution (#6) already exists — this completes it with enforcement. A shared halt primitive also makes the watchdog cheap. |
| 2 | **Secrets: no-secret-in-log invariant + minimal store** | ✅ Yes | The ONE irreversible-when-late: `ActivityScheduled` persists the full `Activity` into the append-only, forever-replayed log. Must be in place BEFORE any runner consumes a key. |
| 3 | **Liveness watchdog (signals + auto-pause)** | ⚠️ Partial | Valuable once a real D2 runs unattended, but "notify owner" can't ship while messaging is deferred. Build the deterministic signal-derivation + auto-pause (reuses P1 halt); hold the notify. |

## Core primitive — `src/guard.rs`

A new module holding the projection-based checks. Two, orthogonal, mechanisms:

1. **Budget** — derived straight from spend (never decreases), so a halt is a
   permanent, always-recomputed state (NOT resumable by ResumeWork; only a higher
   budget limit un-halts it). Owner-set via `POST /api/budget`
   → `BudgetSet { limit_usd, warn_at }` event → `proj.budget`.
2. **Pause** — a resumable flag (watchdog / owner). `WorkPaused { reason, by }` /
   `WorkResumed { by }` events → `proj.paused: Option<PauseInfo>`.

```rust
pub enum BudgetStatus { Disabled, Ok, Warn { fraction }, Halted { fraction } }
pub fn budget_status(proj) -> BudgetStatus          // warn_at (default 0.80), limit
pub fn is_paused(proj) -> bool
pub fn llm_dispatch_allowed(proj) -> Result<(), String> // refuses if paused OR budget-halted
```

**Enforcement points (both consult `llm_dispatch_allowed`):**
- `pm::respond` — before `orch.plan()` (the LLM/provider call): if blocked, skip
  the call (no spend). This is the real, live enforcement once D2 wires.
- `executor::execute` — before running a side-effecting `ActivityKind`
  (`LlmCall`/`GitPush`/`Shell`, never `Inline`): the durable path is guarded too.

**Owner surface:** `POST /api/budget` (owner; required), `POST /api/resume`
(owner; clears a pause). Watchdog auto-pause is internal. Any new route is added
to the `web_boot.rs` regression list (pitfall #9).

## Feature 2 — Secrets

- **Invariant + test (the hard part):** an `Activity` holds a secret *name/key*,
  never a *value*; the event log and `result_ref` never carry a secret; runner
  injects values at call time. Enforced by a `guard::assert_no_secret(activity)`
  helper called at schedule/execute + a test that an attempt to persist a value
  that "looks like" a secret is rejected.
- **Minimal store:** `src/secrets.rs` — a per-project store OUTSIDE the event log
  (values live on disk, gitignored; never in the log), read through a
  `secrets.get(name) -> Result<Option<String>>` runner seam, wired behind the
  existing `ActivityRunner` trait. Kept minimal: no request-scoped ceremony —
  the harness performs side effects, so the *runner* holds the key and its value
  never enters a prompt/context. Full vault complexity is deferred.

## Feature 3 — Watchdog (partial)

- A `ReconcilePass`-style monitor (`src/watchdog.rs`) that derives liveness
  signs deterministically from the event log: no events in N hours, repeated
  error pattern > 3, escalation-frequency above threshold, spend acceleration.
- On a detected stall it emits `WorkPaused { reason: "watchdog: no progress" }`
  (the P1 pause mechanism) → auto-pause. Logs an alert event for future surfacing.
- "Notify owner" is HELD (messaging deferred) — the pause itself is the
  self-actuating protection while Ben is away.

## Test + gate discipline

- TDD: new events/actions get reducer + gate + route tests; everything runs
  through `actions::validate`; new `/api/*` added to `web_boot.rs`.
- Each feature is its own commit (compiles + green on its own, pitfall #14);
  commit AND push after each (owner standing instruction).
- Full `make` gate (fmt → clippy -D warnings → test → build) before each commit.
