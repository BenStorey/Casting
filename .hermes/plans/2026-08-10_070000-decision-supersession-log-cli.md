# Decision Supersession + `cast log` / Event Integrity — Plan

> **For Hermes:** implement task-by-task with TDD; commit after each.

**Goal:** Two roadmap items, both deterministic and on the state-core path:

1. **Decision lifecycle maturity — supersession** (roadmap item 3): decisions
   can be superseded (never deleted, history preserved) — mirroring how
   directives supersede. This is the deterministic part of "anti-thrash /
   superseded / re-opened" decisions.
2. **Event-stream integrity + `cast log` CLI** (roadmap item 4): a CLI to
   inspect the raw event log and a verifier that asserts stream invariants
   ("no gap in sequence", "a DecisionMade always follows a DecisionProposed").

**Architecture (SEMANTIC_EVENTS.md):** events are the only authority; decisions
and projections are derived. Supersession = mark old decision `Superseded` with
a pointer to the new one, preserving both. The CLI reads the append-only store
directly and checks structural invariants.

---

## Feature 1 — Decision supersession

### Model (`src/projection.rs`)
- Add `DecisionStatus::Superseded`.
- `Decision` gains `superseded_by: Option<String>` (which decision replaced it).

### Event + reducer
- `EventType::DecisionSuperseded { superseded_by }` → status = Superseded, set
  `superseded_by`.
- Reducer in `apply()` (find decision by aggregate.id).

### Action + gate (`src/actions.rs`)
- `PmAction::SupersedeDecision { decision_id, by_decision_id }` → emits
  DecisionSuperseded.
- `validate`: decision exists, `by` is a real decision, and neither is already
  superseded (else `DecisionNotFound` / `NotOpen`-style error).

## Feature 2 — `cast log` + integrity

### `cast log` CLI (`src/main.rs`, `src/replay.rs` new)

```text
cast log --db <events.db> [--project <id>] [--tail N]   # human-readable dump
cast log --db <events.db> --verify [--project <id>]     # check invariants
```

- `replay::dump(store, project)`: rows `#seq event_type actor aggregate data` to
  stdout (compact JSON-ish) — the raw, authoritative history.
- `replay::verify(store, project) -> Vec<String>` problems:
  - **sequence gaps**: read all seqs, assert contiguous from 1..max.
  - **DecisionMade follows DecisionProposed**: a project must not contain a
    `DecisionMade` with no preceding `DecisionProposed` for the same
    aggregate.id.
  - Assert any others cheaply enumerable (e.g. TaskCompleted requires
    TaskCreated for that task id).

### CLI wiring
- Extend `main()` with `log` subcommand (`cast log ...`).

---

## File changes

| File | Change |
|---|---|
| `src/projection.rs` | `DecisionStatus::Superseded`, `Decision.superseded_by`, reducer |
| `src/event.rs` | `EventType::DecisionSuperseded` |
| `src/actions.rs` | `SupersedeDecision` action + gate + to_events |
| `src/replay.rs` (new) | `dump(store, project)`, `verify(store, project)` integrity checks |
| `src/main.rs` | `cast log` + `cast log --verify` subcommands |
| `src/lib.rs` | `pub mod replay;` |
| `tests/decisions.rs` (new) | supersession reducer + gate tests |
| `tests/replay.rs` (new) | dump output, verify clean, verify catches gap & orphan DecisionMade |

---

## Tasks (TDD, commit after each)

1. **Decision supersession (model + reducer + action + gate)** — commit:
   `feat(decision): supersede a decision (history preserved, links superseded_by)`.
2. **`replay` module + `cast log` CLI** — `dump` + `verify`; wire `cast log`.
   Commit: `feat(cli): cast log — dump raw history + verify stream invariants`.
3. **Docs** — roadmap items 3 & 4 marked, repo layout, test count.

---

## Tests / validation

- `tests/decisions.rs`: supersede marks old Superseded + links `superseded_by`,
  keeps both; gate rejects superseding a missing / already-superseded decision;
  end-to-end via drive_pm.
- `tests/replay.rs`: dump prints each event; verify on a clean log = empty
  problems; artificially introducing a sequence gap OR an orphan DecisionMade
  → verify reports it.
- Full gate `make` (fmt→clippy→test→build). Currently 103 tests.

---

## Risks / tradeoffs

- **Anti-thrash scope:** we implement the deterministic supersession + integrity
  foundations now. The *reactive* anti-thrash (PM deciding *when* to supersede /
  not re-proposing) needs the LLM/PM reasoning (deferred, D2).
- **`cast log --db` reads a raw events.db:** uses `SqliteEventStore::open` +
  `read_since(0)`. Unknown/absent project returns empty. `--project` defaults to
  the store's present project(s); if multiple, list them.
- **Invariants are advisory, not DB-enforced:** `verify` reports drift rather
  than blocking appends. DB-enforced constraints are a later hardening.