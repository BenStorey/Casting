# Graph / Transition Spine

**Date:** 2026-08-13
**Status:** Approved (Ben: "go all in, no backwards compatibility, core as safe/optimal/deterministic as possible")

## Why

The task/decision lifecycle IS the core of Casting. Everything routes through it.
This makes the lifecycle *explicit*: derived states, a single written transition
contract, decomposition/joins for parallel work, and "why in this order"
provenance. It is the **deepening of the one true concept**, not a new bolt-on —
and it rebalances the codebase back toward Category-A (core) work after a run of
peripheral surfaces.

## Invariants held (do not break)

- Event log is the ONLY authority. Graph, states, groups, transitions are all
  **derived, deterministic projections** — never stored authoritative, never an
  LLM decision.
- The join rule ("have all branches resolved?") is a **structural reducer rule**,
  not a policy judgment. The PM may judge *at* the join node, not *whether* to join.
- No backwards compatibility: cut, don't alias.

## Event model additions (the only schema change)

1. `Task.parent_id: Option<String>` (types.rs, serde-default None). Set in the
   `TaskCreated` reducer from the event's `parent_id` field.
2. `EventType::TaskDecomposed` — records decomposition intent for provenance:
   `{ parent, children: Vec<String> }`.
3. `PmAction::DecomposeTask { parent, children: Vec<TaskSpec> }` where
   `TaskSpec { id, title, kind }`. `to_events` emits one `TaskDecomposed` +
   one `TaskCreated` per child (each carrying `parent_id`). `validate`: parent
   exists, child ids fresh/unique. **Grouping = a parent task + its children; the
   parent IS the join point** (matches the auth-example tree; no separate group
   entity needed).

## Derived TaskState (types.rs) — fully derived, alongside TaskStatus

```
enum TaskState { Queued, Working, InReview, AwaitingHuman, Rejected, Done }
```
`Projection::task_state(task)` maps status + review + open-decision:
- Done → Done · Backlog → Queued · Working → Working (or AwaitingHuman if an
  open owner-decision targets this task) · InReview → InReview (or Rejected if
  review `approved=false`) · Blocked → AwaitingHuman.
- `MergeReady` is intentionally NOT a persistent state: approve→Done already in
  the reducer; the graph shows "InReview → Done (approve)" as a transition.

## src/graph.rs — the spine

- `Transition { id, label, from, to, action, gate: fn(&Projection,&Task)->Result<(),String> }`
  + static `TABLE` + `transitions_for(state, proj, task) -> Vec<&Transition>`.
  **Single contract, three consumers**: PM prompt ("valid exits from X: …"),
  a validation/debug check, and the dashboard.
- `GraphNode { task_id, title, kind, status, state, assignee, parent_id,
  children, awaiting_human, chain (state-derived causal steps), transitions }`.
- `GraphGroup { parent_id, title, children, done, remaining, resolved,
  blocked_by }` — join rule: `resolved` iff every child is terminal (Done).
- `GraphView { nodes, groups, active, blocked, done, total }`.
- `Projection::children_of(id)`, `Projection::task_state(task)`,
  `Projection::graph() -> GraphView`, `Projection::pm_task_context(id)`
  (narrow D2 prompt data).

## Web + SPA

- `GET /api/graph` → `GraphView`. Register in web.rs + router-boot test.
- SPA: a Graph view on Overview — groups/tree, per-node state badge + assignee +
  valid-next-transitions + blocked marker, and an "awaiting human" panel. Read
  from `store.graph` (new fetch), no re-projection client-side.

## Build order (each stage committed + pushed)

1. Event model: parent_id + TaskDecomposed + DecomposeTask (action/gate/events).
2. graph.rs: TaskState + transition table + GraphView + join + pm_task_context.
3. `/api/graph` + router-boot test.
4. SPA Graph view.
5. Full `make` gate green; commit + push.

## Explicitly out of scope (now)

Real-D2 LLM wiring (deferred per roadmap). A separate group entity / DAG
multi-parent (add only when a real case needs it). MergeReady lifecycle state.
