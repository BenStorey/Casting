# Plan: Persistent per-consultant worktrees (2026-08-14)

## Problem

Worktrees are currently created per-task and torn down when the task is done.
The tear-down destroys the warm `target/` directory, forcing every new task
to do a cold rebuild. This wastes time and token budget on compilation.

## Design

Each consultant owns N persistent worktrees (slots). Worktrees are never
destroyed — only reset between tasks. The warm `target/` stays across tasks.

### 1. Consultant config: `max_concurrent`

In the consultant's TOML package (`cast/*.toml`):

```toml
[max_concurrent]
max_tasks = 2  # default 1
```

Controls how many worktrees the consultant gets at setup. The casting harness
pre-provisions N worktrees per consultant at cast-seeding time.

### 2. Worktree struct (per-consultant-slot, not per-task)

```rust
pub struct Worktree {
    pub consultant: String,       // "lead-programmer"
    pub slot: usize,              // 0..max_concurrent-1
    pub task_id: Option<String>,  // None = free, Some = bound to a task
    pub branch: String,           // current branch (main or casting/task-xxx)
    pub path: String,
    pub cargo_target_dir: String,
    pub port: u16,
}
```

Each slot gets its own stable port (`BASE + consultant_offset + slot`).

### 3. Worktree provisioning (at cast seeding or setup)

`CastProvisionWorktrees` action fires N `WorktreeProvisioned` events per
consultant. Each carries `{ consultant, slot, branch: "main", port, path }`.
No `worktree_id` is tied to a task yet (`task_id` = null in the projection).

`WorktreeRemoved` is removed: worktrees never die. The reconciler's
`StaleWorktreePass` now resets (rather than removing) completed worktrees.

### 4. Worktree binding (at task assignment)

When the PM dispatches `assign_task {assignee, task_id, merge_authority}`,
the platform automatically finds the consultant's first free slot
(task_id=None). If none are free, the assignment is rejected with
`PolicyError::NoFreeWorktree`.

A new event `WorktreeBound` records the binding:
`{ consultant, slot, task_id, branch }`.

The projection updates the Worktree: `task_id = Some(task_id)`,
`branch = "casting/<task_id>"`.

The executor runs `git checkout main && git pull origin main && git branch
casting/<task_id>` inside the existing worktree dir (warm target preserved).

### 5. Worktree release (at task done/merged)

When a task reaches Done/Merged (and commits are merged or a ChangeSet is
merged), the reconciler releases the worktree:

- A `WorktreeReleased` event clears the binding:
  `{ consultant, slot, task_id }`
- The projection sets `task_id = None`, `branch = "main"`
- The executor runs `git checkout main && git reset --hard origin/main && git
  clean -fd` inside the worktree dir (target/ is untouched)

The `StaleWorktreePass` does this instead of removing worktrees.

### 6. PM routing

The PM prompt includes per-consultant capacity:
```
lead-programmer: 1/2 tasks assigned (1 available worktree)
```

When the PM assigns a task, it simply specifies the consultant id (as before).
The platform picks the free slot automatically. If no slot is available,
the PM gets back `assign_task rejected: no free worktree for lead-programmer`
and can wait or hire another consultant.

### 7. Events added / removed

- **New:** `WorktreeBound { consultant, slot, task_id, branch }`
- **New:** `WorktreeReleased { consultant, slot, task_id }`
- **Removed:** `WorktreeRemoved` (never needed again)
- `WorktreeProvisioned` stays but now carries `consultant` + `slot` instead
  of just `task_id`. Backward-compatible: the projection reads from the new
  fields and defaults the old ones.

### 8. Projection changes

```rust
pub worktrees: Vec<Worktree>,  // still one vec, still in projection
```

But `Worktree.task_id` is now `Option<String>` instead of `String`.
All apply handlers updated. The projection's `worktrees` query helpers
(used by AgentContext / OperatingModel) filter by consultant.

## Implementation order

1. `Worktree` struct: `task_id: Option<String>`, add `consultant` + `slot`
2. `WorktreeProvisioned` event: add `consultant` + `slot` fields
3. `WorktreeBound` + `WorktreeReleased` events
4. `max_concurrent` config on consultant packages
5. Pre-provision worktrees at setup (instead of per-task)
6. Auto-bind on assign_task, auto-release on done/merged
7. Reconciler: reset instead of remove
8. Wake events updated
9. Tests updated, full suite green