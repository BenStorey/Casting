# Plan: Event-log archival / memory decay (2026-08-14)

**Problem:** The event log grows without bound. Every agent context rebuild loads
the full projection, which includes every task that ever existed, even ones
closed weeks ago. This wastes context tokens on stale history.

**Goal:** Replace old, terminal entities with compact archival summaries so
agents only see active/recent state. Never delete event data (provenance
survives for `task_chain` / `/api/provenance` / the raw event stream).

## Design

### 1. New event: `EntityArchived`

`event.rs`:
```
EntityArchived { entity_kind, entity_id, summary, result }
```
- `entity_kind`: "task", "decision", "opinion", "observation", "risk"
- `entity_id`: the original entity's aggregate id
- `summary`: human-readable one-liner of what happened
- `result`: structured outcome (e.g. "done", "superseded_by:decision-2", "resolved", "materialized")

### 2. Track terminal-at sequence

Add `archived: bool` to Task, Decision, Opinion, Observation, Risk in the
projection. Set `true` on `EntityArchived`. All `apply()` handlers in
`Projection` check `archived` before adding to an active list:
- Task: skip if `task.archived` (it's in the archive, not in active state)
- Decision: same
- Opinion: same
- Observation: same
- Risk: same

### 3. ArchivedRecord struct

In `projection.rs`:
```
pub struct ArchivedRecord {
    pub entity_kind: String,   // "task" | "decision" | "opinion" | "observation" | "risk"
    pub entity_id: String,     // original aggregate id
    pub summary: String,       // human-readable one-liner
    pub result: String,        // structured outcome
    pub archived_at: String,   // when it was archived (from event)
    pub archived_by: String,   // who archived ("reconciler" by default)
    pub source: String,        // the entity's original source/kind
}
```

### 4. Projection field

```
pub archived: Vec<ArchivedRecord>
```

This is the compact history. It replaces the individual entities in the active
lists. The archive is itself foldable from `EntityArchived` events.

### 5. Reconciler pass

A new `ReconcilePass` in `reconciler.rs`:
```
pub struct ArchivePass {
    /// Threshold: how many events since status became terminal before archiving.
    pub threshold: u64,
}
```

On each run:
1. Get current sequence from projection
2. For each entity in `tasks`, `decisions`, `opinions`, `observations`, `risks`
   that is terminal (Done / Superseded / Resolved / inactive):
   - If `!archived` AND current_sequence - entity.terminal_at > threshold:
     - Build a summary from the entity's data + any review/result info
     - Fire `EntityArchived` event
     - Set `archived = true` on the entity
3. The next projection build folds the archived entities into `archived` and
   out of the active lists.

### 6. AgentContext filtering

`context.rs` (`build_for_actor`): when populating `my_tasks`, `open_risks`,
`open_decisions`, `active_observations`, etc., skip any entity where
`archived == true`. This is the actual context-saving mechanism — agents never
see archived entities in their daily planning context.

### 7. OperatingModel surface

`mental.rs` `OperatingModel` gains an `archive` section listing recent
archived records with summaries, so the owner can still see "what was done
recently" at a glance.

## Order of implementation

1. `EntityArchived` event type + `ArchivedRecord` in projection
2. `archived` bool on Task, Decision, Opinion, Observation, Risk
3. Projection build filters: skip archived entities in apply() handlers
4. `ArchivePass` reconciler with threshold-based archive trigger
5. AgentContext filtering for archived entities
6. OperatingModel archive section

This keeps the implementation incremental: each step compiles and tests pass
before the next step adds behavior.