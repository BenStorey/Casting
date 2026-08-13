# Project Directives — Implementation Plan

> **For Hermes:** implement task-by-task with TDD; commit after each task.

**Goal:** Add **Governance** to Casting (per `docs/INTENT.md`): policies,
constraints, principles, practices, and preferences as **first-class,
event-sourced project state** — not prompt text. Directives are authoritative,
selectively surfaced per agent, and enforced by the same delegated-authority
machinery we've built.

**Architectural principle (INTENT.md):** Casting has three kinds of knowledge:
**Intent** (what/why), **Governance** (how/rules), **State** (current), and
**History** (why/when). Directives are *governance* — durable domain objects
whose lifecycle is recorded by events, reduced into the projection, and
relevant-filtered per agent by a context resolver.

> **"Agents interpret, the system records"** — the *creation* of a directive may
> need the PM/LLM to interpret owner language, but the directive's lifecycle
> transitions are deterministic reducers, and editing them flows through the
> authority gate (agents can *propose*, only authorized actors *change*).

---

## Current context

- `Projection` already carries first-class semantic objects (`risks`,
  `assumptions`, `constraints`) reduced from events; directives extend this.
- `PmAction` + `actions::validate` (pure gate) + `actions::to_events` is the
  command/LLM seam; gate rejects invariant-violating actions.
- `PolicyError` has ownership/authority variants; add directive ones.
- House style: curated `EventType`, typed enums, pure reducers/gates, clippy
  -D warnings, fmt clean, conventional commits. `make` = one-step gate.

## Design

### 1. Directive model (`src/directive.rs`, new module)

```rust
pub enum DirectiveKind { Policy, Constraint, Principle, Practice, Preference, Objective }
pub enum DirectiveStrength { Required, Strong, Recommended }  // ordered
pub enum DirectiveStatus { Active, Suspended, Superseded, Expired }

pub struct Directive {
    pub id: String,
    pub kind: DirectiveKind,
    pub statement: String,
    pub scope: Vec<String>,       // areas it applies to (e.g. "engineering", "architecture")
    pub strength: DirectiveStrength,
    pub status: DirectiveStatus,
    pub created_by: String,
    pub supersedes: Option<String>, // id of directive it replaced (INTENT supersession)
}
```

- `Directive::new(...)` → status `Active`.
- **Strength hierarchy** for conflict resolution (INTENT): `Required` >
  `Strong` > `Recommended`. Pure `PartialOrd` on the enum, plus a
  `strength_rank()` used by the resolver.
- Register `pub mod directive;` in `lib.rs`.

### 2. Events + reducer (`src/event.rs`, `src/projection.rs`)

Curated lifecycle events (per INTENT, kept deliberate):

```text
ProjectDirectiveCreated    { kind, statement, scope, strength, created_by, supersedes? }
ProjectDirectiveSuspended  {}                      // status -> Suspended
ProjectDirectiveResumed    {}                      // status -> Active
ProjectDirectiveSuperseded { superseded_by }       // status -> Superseded
ProjectDirectiveExpired    {}                      // status -> Expired
```

- `Projection.directives: Vec<Directive>`; default empty.
- Reducers in `apply()`: created → push `Active`; suspended/resumed/superseded/
  expired → set status (idempotent find-by-id).
- Kept minimal (no `Modified` for round one — a supersede is the explicit way to
  change a directive, matching INTENT's supersession bias over silent edits).

### 3. Actions + authority gate (`src/actions.rs`)

```rust
PmAction::CreateDirective   { id, kind, statement, scope, strength, supersedes? }
PmAction::SuspendDirective  { directive_id }, PmAction::ResumeDirective { directive_id }
PmAction::SupersedeDirective{ directive_id, by_directive_id }
PmAction::ExpireDirective   { directive_id }
```

Gate (`validate`) rules — this is the delegated-authority part:
- **Only the owner (or the PM/system) may create or change a directive.** A
  plain agent cannot — it can only raise an `Observation` (propose), which the
  PM evaluates. Enforced via `who`.
- Create/Supersede carry a `supersedes` — must reference an existing, active
  directive (else `DirectiveNotFound` / `DirectiveNotActive`).
- Suspend/Resume/Expire/Supersede require the directive to exist.

### 4. Context resolver — selective surfacing (`src/directive.rs`)

The INTENT payoff: directives exist once, surfaced per agent:

```rust
pub fn relevant<'a>(projection, areas: &[&str]) -> Vec<&Directive>
// filters Active directives whose scope intersects `areas`,
// sorted by strength (Required .. Recommended), then by recency.
```

Pure, deterministic, testable. The scripted PM's prompt/context can call it.

### 5. Wire into plan + `/api/state`

- `Projection.plan()` (or `/api/state`) surfaces `active_directives` — the
  current governing rules — alongside objectives/priorities/risks.
- The PM's boarding plan creates a couple of seed directives (e.g. "TDD is
  required", "No backwards-compatibility requirement") to demonstrate them as
  first-class state.

---

## File changes

| File | Change |
|---|---|
| Create `src/directive.rs` | `DirectiveKind/Strength/Status`, `Directive`, `relevant()` |
| Modify `src/lib.rs` | `pub mod directive;` |
| Modify `src/event.rs` | 5 directive lifecycle events |
| Modify `src/projection.rs` | `directives` field + reducers; surface in plan/api-state |
| Modify `src/actions.rs` | 5 actions + authority gate + `to_events` |
| Modify `src/pm.rs` | (optional) seed directives on boarding |
| Create `tests/directives.rs` | reducer, gate, relevance tests |

---

## Tasks (TDD, commit after each)

1. **Directive model** — `DirectiveKind/Strength/Status` + `Directive` +
   `strength_rank` + `relevant()`. Register module. Test: enums, ordering,
   `relevant` filtering/sorting. Commit: `feat(directive): model + context resolver`.
2. **Events + reducer** — 5 events, `Projection.directives`, reducers. Test:
   created→Active, suspended/resumed/superseded/expired status transitions,
   idempotent. Commit: `feat(projection): reduce directive lifecycle events`.
3. **Actions + authority gate** — 5 actions, `validate` (owner/PM only; valid
   supersedes target), `to_events`. Test: owner/PM allowed, plain agent
   rejected; supersede to existing-active only. Commit:
   `feat(actions): directive commands through the authority gate`.
4. **Surface + docs** — plan/api-state `active_directives`; optionally seed in
   PM boarding; HANDOFF roadmap (governance layer), repo layout, test count.
   Commit.

---

## Tests / validation

- `tests/directives.rs`: model enums/ordering/relevance; reducer lifecycle;
  gate authority (owner/PM vs agent) + supersede validation; end-to-end via
  `drive_pm`.
- Full gate: `make` (fmt→clippy -D warnings→test→build). Currently 89 tests.

---

## Risks / tradeoffs

- **Authority scope:** for round one only Owner and the PM/system may change
  directives. The PM may *propose* and, if the directive's strength/kind
  warrants (later: a decision-policy mapping), escalate to the owner — matches
  the INTENT "Agent → observation → PM → policy check → owner if necessary".
  YAGNI: we don't build Proposal/Request events now; an Observation already
  exists for that.
- **No `Modified`:** superseding a directive (id → id) is the explicit way to
  change it, keeping history clean. A blanket `Modified` is deferred unless a
  real need appears.
- **Scope is free-text strings:** deterministic matching on area tokens; a
  richer taxonomy is a future hardening.