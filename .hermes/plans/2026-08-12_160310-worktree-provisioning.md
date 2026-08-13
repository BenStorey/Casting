# Worktree Provisioning — isolated per-consultant workspaces — Implementation Plan

> **Status: DONE (2026-08-12).** All 8 tasks implemented, committed, and pushed
> (216 tests, clippy 0, fmt clean). Commits:
> `333e573` workspace primitives → `f54a458` event+projection → `24b99fb` gate →
> `0b94d60` port allocator → `89a021e` PM summon wiring → `f6db409` commit
> surface → `368f926` reconciler prune → `68dde69` context/operating-picture
> surface. See `references/worktree-isolation.md` for the finished design + pitfalls.

> **For Hermes:** implement task-by-task with TDD; commit after every task; push at the end.

**Goal:** When a consultant is summoned (a task is assigned to them), the
platform deterministically provisions an **isolated workspace**: a git worktree
on its own branch, with its **own private build target** and its **own API port**
so concurrent consultants cannot collide. Isolation is a *platform property*, not
an agent behavior — the LLM is never asked to "remember to use a worktree"; it is
handed a ready, isolated workspace in its context.

Directive: build the deterministic surface FIRST (⚡ 2026-08-10). No LLM. This is
exactly that kind of slice.

## Why / principle

> **Isolation should be a property of how the platform hands work to a
> consultant, not a behavior the consultant must exhibit.**

ADDENDUM §20 already declares the invariant (work must be isolated, inspectable,
reversible, reviewable before it touches protected state) — today it's prose.
This makes it structural. Matches the existing ChangeSet/observer/gate
architecture; the observer currently *guesses* task_id from branch-name
convention (`derive_task_id`) — provisioning records the mapping exactly instead.

## Owner-added requirements (Ben, 2026-08-12)

1. Each worktree gets its **own private Rust build target** (`CARGO_TARGET_DIR`)
   so concurrent consultants compiling don't stomp each other's `target/`.
2. Each worktree gets a **different API port** so each consultant's dev server
   can run without colliding.
3. (core) Always worktree + own branch at summon.

## Architecture

### 1. Provisioning lives in `Workspace` (the pinned git runner)

`src/workspace.rs` gains:
- `provision_worktree(&self, task_id, slug) -> Result<ProvisionedWorktree>`
  which runs (through `git_command`, pinned to the repo):
  - `git worktree add <repo>/.casting/worktrees/<task_id> -b casting/task-<id>-<slug>`
    (branch created off current HEAD; `main` not touched).
  - Allocates the **private build target**: `<repo>/.casting/worktrees/<task_id>/target`
    (a `CARGO_TARGET_DIR` the agent's build commands will use).
  - Allocates the **API port**: first free port from a per-project range
    (configured base, e.g. `8081+`; tracked in the projection so it's auditable
    and the reconciler can free it on cleanup).
- `remove_worktree(&self, task_id)` → `git worktree remove` + prune.
- `git_command_for(&self, worktree)` — a runner variant that pins
  `GIT_WORK_TREE`/`GIT_DIR` to the *worktree* (worktrees share `.git` but have
  their own tree), used by the agent git surface.

`ProvisionedWorktree { task_id, branch, path, cargo_target_dir, port }`.

### 2. A first-class event + projection state

`EventType::WorktreeProvisioned` (System actor, aggregate kind `worktree`,
id `wt-<task_id>`):
```json
{ "task_id", "branch", "path", "cargo_target_dir", "port" }
```
Reducer creates a `Worktree` entry in `Projection.worktrees` (Vec), and
**auto-creates the ChangeSet in `Open`** (task_id + branch) so the mapping is
exact (no `derive_task_id` guessing). `Projection.worktrees: Vec<Worktree>`.

### 3. The gate + action surface

- `PmAction::ProvisionWorktree { task_id, slug }` — `validate`: task exists,
  agent is a hired consultant (not owner), no worktree for that task yet
  (else `DuplicateEntity`). `to_events` → `WorktreeProvisioned`.
- **Fail-closed gate:** `StartTask` requires a provisioned worktree for the task
  (assert the workspace exists) — structurally can't work un-isolated.
- **Agent git surface (thin):** `PmAction::CommitToChangeSet { task_id,
  message }` — `validate`: task is assigned to the acting agent, a worktree
  exists, worktree is clean-pushable. Executes `git add -A && git commit`
  *in the worktree* via `git_command_for`. Reducer tallies commits into the
  ChangeSet (and the observer's `CommitObserved` will also note them — reconcile;
  the ChangeSet is the authority). Keep this minimal — the agent owns content,
  the platform owns isolation.

### 4. Wiring into the PM "summon"

In `pm.rs` `plan_*`, when a task is assigned to a hired consultant, the PM
emits `ProvisionWorktree` first (before `StartTask`), so onboarding/assignment
now creates the isolated workspace. (Scripted for now; the seam is ready for D2.)

### 5. Reconciler cleanup

The drift reconciler's every-N-events pass also **prunes** stale worktrees: a
`Worktree` whose task is `Done`/`Merged` has its worktree removed
(`git worktree remove` + prune) and the port freed. Deterministic, cursor-gated.

### 6. Surface in context

`context_for(agent)` / `/api/model` includes the agent's worktree (`path`,
`cargo_target_dir`, `port`) so the LLM (later) and the agent's build/dev commands
know exactly where/how to work. Persona/operating-picture show worktree state.

## Files

| File | Change |
|---|---|
| `src/workspace.rs` | `provision_worktree`, `remove_worktree`, `git_command_for`, `ProvisionedWorktree` |
| `src/event.rs` | `EventType::WorktreeProvisioned` |
| `src/types.rs` | `Worktree` struct + `Projection.worktrees` |
| `src/actions/action.rs` | `ProvisionWorktree`, `CommitToChangeSet` |
| `src/actions/policy.rs` | gate checks (task exists, consultant, no dup; StartTask requires worktree) |
| `src/actions/events.rs` | `to_events` for both |
| `src/projection.rs` | reducer for `WorktreeProvisioned`; auto-ChangeSet; `worktrees()` |
| `src/workspace.rs` / `src/port.rs` | deterministic port allocator (base + per-project tracking) |
| `src/pm.rs` | summon path emits `ProvisionWorktree` before `StartTask` |
| `src/reconciler.rs` | prune done/merged worktrees + free ports |
| `src/context.rs` / `src/mental.rs` | surface worktree in agent context + operating picture |
| `tests/worktree.rs` (new) | provisioning, gate, commit-surface, collision-avoidance, cleanup |
| docs | HANDOFF.md, ADDENDUM §20 note, skill reference |

## Tasks (commit each)

1. `Workspace` worktree provisioning + `git_command_for` + tests (path/branch/target/port via a temp repo).
2. `WorktreeProvisioned` event + `Worktree` projection type + reducer (auto-ChangeSet) + tests.
3. `ProvisionWorktree` action + gate (fail-closed: StartTask requires worktree) + tests.
4. Port + build-target allocation (deterministic, no collisions) + tests.
5. `CommitToChangeSet` agent git surface + tests.
6. PM summon wiring (provision before start) + tests.
7. Reconciler prune + tests.
8. Context/operating-picture surface + docs + count.

## Validation

`make` (fmt + clippy -D warnings + all tests + build) green after each task.
Live-verify: `cast run <dir>` on a fresh project, assign a task, confirm a
worktree exists on its own branch with distinct target + port; commit through
`CommitToChangeSet`; confirm the ChangeSet tallies; let the reconciler prune.