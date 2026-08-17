# Casting codebase conventions (reference)

Ground your survey/design work in these project conventions.

## Event-sourced architecture
- **Source of truth is the event log** (`store`), never derived state. The
  projection is a derived view, rebuilt by replay — treat it as read-only
  input, not something to mutate.
- **Events are immutable facts**: an `Event` records what happened (project,
  actor, `EventType`, aggregate, data). Adding a new behaviour means adding a
  new event/action, not editing history.
- **Policy gate** validates proposed actions before they persist. If an action
  is rejected, the model over-planned — align the plan with what the gate
  allows (the action vocabulary is the contract).

## Module layout (2026-08-16 restructure)
`src/` is organised into domain directories, each exposing a `mod.rs`:
  `actions/` (validated action vocabulary + policy/director/owner), `event/`
  (event model, replay, archival/scrub), `pm/` (Project Manager loop: control,
  guard, planning, policy, reconciler, triage), `projection/` (graph/port/
  derived state), `runtime/` (channel, context, executor, mental, orchestrator,
  persona, telegram, wake, watchdog), `store/` (event store + pg/sqlite
  backends), `types/`, `workspace/` (auth, cast, merge, setup, worktrees).

## Worktree discipline
- Assigned implementation happens in the consultant's **persistent worktree**
  (their own branch + private `CARGO_TARGET_DIR`), never the shared checkout.
- The `Worktree` in context carries `task_id`, `branch`, `path`,
  `cargo_target_dir`, `port`.

## Testing
- `tests/` holds integration tests; behaviour changes that make a test assert
  stale state mean *update the test to match the new behaviour*, never contort
  the code to keep an old test green.
- Add regression tests with fixes and features.