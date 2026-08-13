# State-Core Maturity: Decision Audit, Semantic Objects, Snapshots — Plan

> **For Hermes:** implement step-by-step with TDD; commit after each task.

**Goal:** Nail the event/state/audit architecture before dogfooding. Three
steps: (1) a **decision audit / provenance** view, (2) **semantic state objects**
(Risk / Assumption / Constraint), (3) **snapshots** as a pure optimization.
Per `docs/SEMANTIC_EVENTS.md`: events are mutations, projections are state,
snapshots are never a source of truth.

**Tech Stack:** Rust (single binary, no new deps). SQLite (already present) for
the snapshot store. serde for event/storage JSON.

---

## Current context

- `provenance.rs` walks **commit → changeset → task → requirement → decision →
  owner message** (`for_commit`) and task-side (`for_task`), but there is **no
  decision-centric audit**: given a decision, we can't yet answer "who proposed
  it, what class/involvement, who decided it, and why".
- Semantic concepts (Risk/Assumption/Constraint) exist only as prose in
  SEMANTIC_EVENTS.md §8 — not as first-class objects/events/projections.
- `Projection::build` folds the whole log every request. No snapshots.

---

## Step 1 — Decision audit / provenance view

Add a decision-centric read: `for_decision(store, project, decision_id)`
returning who proposed it, its class/involvement, its status/decider, the
owner's note, and the chain back to the initiating owner message:

```rust
pub struct DecisionAudit {
    pub decision_id: String,
    pub subject: String,
    pub class: crate::policy::DecisionClass,
    pub involvement: crate::policy::OwnerInvolvement,
    pub status: String,              // proposed | approved | rejected
    pub proposed_by: String,         // actor
    pub decided_by: Option<String>,  // owner | agent id
    pub owner_note: Option<String>,
    pub owner_message: Option<String>, // the intent that led here
    pub chain: Vec<ProvenanceLink>,    // DecisionProposed → DecisionMade → owner message
}
```

- Pure query over the event log (`read_since`), same style as `for_commit`.
- Wire: `GET /api/provenance/decision/{id}`.
- Tests: proposed-only decision (approved_by None, chain has proposal + owner
  message); decided decision (decided_by owner, note, chain has DecisionMade);
  unknown decision → empty.

## Step 2 — Semantic state objects (Risk / Assumption / Constraint)

First-class semantic objects (SEMANTIC_EVENTS §8) with deterministic reducers
(the *creation* may need the PM/LLM interpretation, but the state transition is
pure). Start with the doc's flagship `Risk`, then `Assumption` + `Constraint`
(both simpler, read-only notes).

**Risk** (full lifecycle — the flagship):
- `EventType::RiskRaised { subject, severity, discovered_by }` → `Risk { status: Open }`
- `EventType::RiskUpdated { status: Resolved|Materialized }` → deterministic reducer
- `Risk { id, subject, severity, status, discovered_by }` in `Projection.risks`
- Actions: `RaiseRisk`, `ResolveRisk` through the gate.
- The plan lists `risks` (open ones at least) alongside priorities.

**Assumption / Constraint** (read-only semantic notes):
- `EventType::AssumptionRecorded`, `EventType::ConstraintRecorded` → vectors in
  `Projection.assumptions`, `Projection.constraints`.
- Actions: `RecordAssumption`, `RecordConstraint` (or direct owner/PM appends).

## Step 3 — Snapshots (pure optimization, never a source of truth)

Add a SQLite-backed **snapshot store** (mirrors `cursor.rs`): `project_id →
(sequence, projection_json)`. `Projection::build` gains an optional snapshot:
load the last snapshot at seq N, apply events (N, latest] on top, fold the
tail. If the snapshot is stale/corrupt/missing, fall back to a full fold —
snapshots are disposable.

- New `snapshot.rs`: `SnapshotStore { save(project, seq, &Projection), load(project) }`.
- `Projection::build_from(store, snapshot_store, project)`: snapshot + tail fold.
  **Semantics unchanged** (same resulting state); correctness tested by
  asserting snapshot+tail == full fold.
- Wire `cast run`/web to use `build_from` (the READ path); snapshots written as
  a side effect of building. `events.db` remains the only authority.
- Tests: snapshot+tail equals full projection; a replayed/deterministic run;
  missing snapshot falls back.

---

## File changes

| File | Change |
|---|---|
| `src/provenance.rs` | `for_decision` → `DecisionAudit` |
| `src/web.rs` | `GET /api/provenance/decision/{id}` |
| `src/event.rs` | `RiskRaised`, `RiskUpdated`, `AssumptionRecorded`, `ConstraintRecorded` |
| `src/actions.rs` | `RaiseRisk`, `ResolveRisk`, `RecordAssumption`, `RecordConstraint` |
| `src/projection.rs` | `risks`, `assumptions`, `constraints` fields + reducers; risk in plan view |
| `src/snapshot.rs` (new) | `SnapshotStore` (SQLite) |
| `src/lib.rs` | `pub mod snapshot;` |
| `src/main.rs`/`web.rs` | READ path uses `build_from` with snapshot store |
| `tests/provenance.rs` | decision-audit cases |
| `tests/semantic_objects.rs` (new) | risk lifecycle + assumptions/constraints |
| `tests/snapshot.rs` (new) | snapshot correctness + fallback |

---

## Tasks (TDD, commit after each)

1. **Decision audit** — `for_decision` + endpoint + tests. Commit: `feat(provenance): decision audit view (who/class, who decided, why)`.
2. **Risk lifecycle** — events + projection + actions + gate + tests. Commit: `feat(semantic): Risk first-class object with lifecycle`.
3. **Assumption + Constraint** — record-only semantic notes + tests. Commit: `feat(semantic): Assumption + Constraint objects`.
4. **Risk in plan view** — plan lists open risks. Commit: `feat(plan): surface open risks`.
5. **Snapshots** — `SnapshotStore` + `build_from` + correctness tests + wire READ path. Commit: `feat(snapshot): SQLite-backed projection snapshots (optimization, never authoritative)`.
6. **Docs** — HANDOFF roadmap updates (steps 1–3 done), test count. Commit.

---

## Risks / tradeoffs

- **Semantic object creation vs state:** we only implement the *deterministic
  reducer + first-class state*; *interpretation* (owner "this is a risk" → a
  RiskRaised event) is the PM/LLM's job, deferred (D2). Tests drive the events
  directly.
- **Snapshots must never drift:** `build_from` is a pure cache; the test
  asserting snapshot+tail == full fold is the guard. If a snapshot is corrupt,
  we discard and full-fold. This is explicitly so snapshots can't become a
  second source of truth.
- **Scope discipline:** Risk is the flagship; Assumption/Constraint are kept
  deliberately simple (record-only) so the pattern is demonstrated without
  over-building.