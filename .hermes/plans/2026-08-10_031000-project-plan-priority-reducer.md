# Project Plan Projection + Priority Reducer — Implementation Plan

> **For Hermes:** implement task-by-task with TDD; commit after every task.

**Goal:** Add the deterministic **Project Plan** as current-state (objectives +
ordered current priorities + open decisions), powered by a typed **priority**
field on tasks reduced from a new `TaskPriorityChanged` event. This is the
"mature the state core" item #2 and the **first dogfooding artifact** — it's the
state object our own roadmap would become, replacing hand-edited `.md`.

**Architecture (per `docs/SEMANTIC_EVENTS.md`):** events are mutations,
projections are state. `TaskPriorityChanged` is a fact; `task.priority` and the
`ProjectPlan` view are *derived deterministically* — no LLM. Definitions:

- **Priority** — a typed, ordered enum (`Critical > High > Medium > Low`).
- **TaskPriorityChanged** — event `{ task_id, from, to }` (mutation).
- **Reducer** — `Task.priority` field, default `Medium`, folded in `apply()`.
- **ProjectPlan** — a derived view on the existing projection: current objective
  (the open requirement), tasks ordered by priority, deprioritized (lowest),
  and open decisions. Deterministic, recomputable, never stored authoritative.

**Tech Stack:** Rust (single binary, no new deps). serde for event JSON.

---

## Current context / assumptions

- `Projection` is folded from the event log in `apply()`; recomputed per request;
  NOT authoritative (event history is). A `plan` field folds naturally here.
- `PmAction` + `actions::validate` + `actions::to_events` is the command path
  (the LLM seam). Adding `SetTaskPriority` lets a producer (scripted now, LLM
  later) mutate priority through the same validated gate.
- `TaskPriorityChanged` is the SEMANTIC_EVENTS design; we implement the
  structured/deterministic part now (the doc reserves *interpretation* — turning
  "auth isn't important" into a mutation — for the PM, which is the LLM's job).
- House style: curated `EventType` enum, pure/testable reducers + gates,
  clippy -D warnings, fmt clean, conventional commits.
- This is Tier-1 dogfooding: it makes current planning-state derivable, so our
  own roadmap can later live as Casting data instead of `.md`.

## Proposed design

### 1. `Priority` enum (`src/plan.rs`, new small module)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority { Critical, High, Medium, Low }   // serde snake_case
impl Default for Priority { Medium }                 // tasks start medium
```

### 2. Event: `TaskPriorityChanged` (`src/event.rs`)

`EventType::TaskPriorityChanged` — payload `{ task_id, from, to }` (from = old,
for history richness; reducer only needs `to`). Curated enum extension.

### 3. Action + gate (`src/actions.rs`)

- `PmAction::SetTaskPriority { task_id, priority }`
- `validate`: task must exist (`TaskNotFound`).
- `to_events`: emits `TaskPriorityChanged`.

### 4. Reducer (`src/projection.rs`)

- `Task { .., priority: Priority }` (default `Medium` on create).
- `apply(TaskPriorityChanged)` → set the task's priority (only `to`).

### 5. ProjectPlan view (`src/plan.rs` or `src/projection.rs`)

Derived from the projection, recomputable:

```rust
pub struct ProjectPlan {
    pub objective: Option<String>,        // the current (open) requirement title
    pub priorities: Vec<PlannedItem>,     // tasks ordered Critical..Low
    pub open_decisions: Vec<String>,      // proposed-decisions awaiting owner
}
pub struct PlannedItem { pub task_id: String, pub title: String, pub priority: Priority }
```

`Projection.plan()` computes it; exposed on `/api/state` as `plan`.

---

## File changes

| File | Change |
|---|---|
| Create `src/plan.rs` | `Priority`, `ProjectPlan`, `PlannedItem`, `Projection` plan derivation |
| Modify `src/lib.rs` | `pub mod plan;` |
| Modify `src/event.rs` | `EventType::TaskPriorityChanged` |
| Modify `src/actions.rs` | `SetTaskPriority` action + validate + to_events |
| Modify `src/projection.rs` | `Task.priority` + reduce the event |
| Modify `src/web.rs` | expose `plan` on `/api/state` (optional; small) |
| Create `tests/project_plan.rs` | reducer + plan derivation tests |
| Modify `tests/vertical_slice.rs` | (optional) PM emits a priority change |

---

## Tasks (TDD, commit after each)

### Task 1 — Priority + ProjectPlan types (`src/plan.rs`)
Add `Priority` (ordered, default Medium, serde) and `ProjectPlan`/`PlannedItem`
types. Register `pub mod plan`.

- Test: ordering `Critical>High>Medium>Low`; default is Medium; serde round-trip.
- Run: `cargo test --test project_plan` — PASS.
- Commit: `feat(plan): Priority enum + ProjectPlan view types`

### Task 2 — TaskPriorityChanged event + reducer
Add the event type; `Task.priority` field (default Medium); fold in `apply()`.

- Test (`tests/project_plan.rs`): append `TaskPriorityChanged(auth, high→low)`,
  build projection, assert `task.priority == Low`; creating a task gives Medium.
- Run: full suite. Commit: `feat(projection): reduce TaskPriorityChanged into Task.priority`

### Task 3 — SetTaskPriority action through the gate
`PmAction::SetTaskPriority` + validate (task exists) + to_events.

- Test (`tests/policy_gate.rs` or plan tests): setting priority on an existing
  task → event emitted; setting on a missing task → `TaskNotFound`.
- Run: full suite. Commit: `feat(actions): SetTaskPriority command through the gate`

### Task 4 — ProjectPlan derivation + wire to state + docs
`Projection.plan()` derives objective/priorities/open-decisions; expose as `plan`
on `/api/state`; update HANDOFF (roadmap item 2 done, dogfooding note).

- Test: build a projection (tasks with mixed priorities + open decision) →
  plan.priorities ordered correctly, objective = open requirement, open_decisions listed.
- Run: full suite, clippy, fmt. Commit.

---

## Tests / validation

- `tests/project_plan.rs` (new): Priority ordering/default/serde; event reducer;
  plan derivation (ordering, objective, open decisions).
- Full gate: `cargo test` (all suites green), `cargo clippy --all-targets -- -D warnings`
  (zero), `cargo fmt` (clean). Currently 67 tests.

---

## Risks / open questions

- **What is the "objective"?** For now: the most recent open `RequirementCreated`
  title. A richer objective is a future `ObjectiveSet` event (out of scope).
- **Deprioritized list:** currently = the `Low`-priority tasks. A semantic
  "deprioritized (owner said so)" marker is a future event; YAGNI now.
- **Plan is state, not prose:** this is the seam where our own roadmap can later
  become Casting data (Tier-1 dogfooding) — not a doc.