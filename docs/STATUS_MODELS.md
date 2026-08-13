# Task status vocabulary — `TaskStatus` ↔ `TaskState` mapping

Two overlapping-but-distinct task status models exist. This is deliberate, but
it was undocumented — this page pins the mapping so consumers never choose an
arbitrary one.

## The two models

**`TaskStatus`** (`src/types.rs`) — the **canonical board position**. A field on
`Task`, projected from lifecycle events and unchanged by review nuance.

- `backlog` — created, not started.
- `working` — assigned and/or started.
- `in_review` — submitted for review.
- `blocked` — paused (awaiting the human).
- `done` — terminal.

**`TaskState`** (`src/graph.rs`) — a **derived, reader-facing** layer *over*
`TaskStatus`, adding the nuance the raw board position can't express. Fully
deterministic; never stored. Used by the graph/transition spine, the PM task
prompt, and dashboard renderers.

- `queued`, `working`, `in_review`, `done` — map 1:1.
- `awaiting_human` — the "blocked" board position, named for what it *means*.
- `rejected` — `in_review` on the board **and** the review verdict was not
  approved (rework due; it still sits `in_review` until resubmitted).

## The mapping

| TaskStatus (board)   | TaskState (derived) | Notes |
|----------------------|---------------------|-------|
| `backlog`            | `queued`            |       |
| `working`            | `working`           |       |
| `in_review`          | `in_review`         | first review / awaiting the reviewer |
| `in_review` (rejected) | `rejected`         | review `approved == false` |
| `blocked`            | `awaiting_human`    | pause node waiting on the owner |
| `done`               | `done`              | terminal |

## Orthogonal: hard-dependency ordering is NOT a status

The graph's hard-dependency ordering (`Projection.dependencies` /
`blocked_by`, the Blocker Test) is a **readiness axis**, not a `TaskState`.
A child with an unsatisfied blocker is surfaced via `GraphNode.blocked_by` /
`PmTaskContext.blocked_by` while its `TaskState` remains `queued`/`working`.
Do not conflate "hard-blocked by an unfinished dependency" with
`awaiting_human` — the latter is a conscious pause waiting on the owner.

## Rules

- **`TaskState` is derived, never stored** — it sits over `TaskStatus`.
- **Never add a state to only one model.** Extend the lifecycle via
  `graph::TABLE` transitions; add a `TaskState` variant only when the board
  position genuinely cannot express the nuance, and update this table.
- **Consumers:**
  - Board (`/api/state`), the watchdog, and the raw projection use `TaskStatus`.
  - Graph, dashboards, and the PM task prompt use `TaskState`.
  - When a consumer needs both, derive `TaskState` from `TaskStatus` via this
    table — never maintain a second source.