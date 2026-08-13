# Persist DecisionPolicy as Domain Events — Implementation Plan

> **For Hermes:** implement task-by-task with TDD; commit after every task.

**Goal:** Make delegated authority **durable and event-sourced** (roadmap item
"mature the core", #1). Today the decision policy is rebuilt in-memory from
`DecisionPolicy::defaults()` and `actions::validate` hardcodes that default — so
the owner's per-class autonomy configuration doesn't exist as history and is not
actually *enforced* from durable state. This makes policy changes first-class
domain events, folds them into the projection, and has the authority gate
consult the event-derived policy.

**Architecture:** New event `DecisionPolicyChanged` (owner-authored, via a `POST
/api/policy` endpoint) → folded into a `policy` field on the `Projection` →
`actions::validate`'s `ProposeDecision` arm checks the claim against
`state.policy` (the event-derived, per-project policy) instead of a hardcoded
default → the scripted PM resolves a proposal's involvement from `state.policy`
so its claim matches what's configured. This makes delegated authority
auditable, durable, and **actually enforced** — the thing Casting lives or dies by.

**Tech Stack:** Rust (single binary, no new deps). serde for event JSON.

---

## Current context / assumptions

- `DecisionPolicy` (`src/policy.rs`) already has `defaults()`, `resolve(class)`,
  `set(class, level)`, and serde (Serialize/Deserialize/PartialEq/Default).
- `Projection` is folded from the event log in `apply()`, is recomputed per
  request, and is NOT authoritative (event history is). A `policy` field folds
  naturally here the same way `decisions` does.
- `actions::validate` currently rejects a `ProposeDecision` using
  `policy::DecisionPolicy::defaults()` — the value to replace with real state.
- `plan_onboard` (`src/pm.rs`) hardcodes each proposal's `involvement` (Ask for
  Database, Pm for testing-library). These happen to equal the builtins; once
  owner overrides exist, the PM must claim what the *configured* policy says or
  the gate (correctly) rejects it.
- House style: curated `EventType` enum, pure/testable gates, clippy -D
  warnings, fmt clean, conventional commits.

## Proposed approach / design

### 1. New event: `DecisionPolicyChanged` (`src/event.rs`)

Description: the owner set/changed the owner-involvement for a decision class.

```rust
EventType::DecisionPolicyChanged
```
Payload (`data`): `{ "class": DecisionClass, "involvement": OwnerInvolvement }`
Aggregate: `kind = "decision_policy"`, `id = <class>`

Actor: `Owner` — configuring autonomy is a human decision about trust. (The PM
can propose changes later if we want; YAGNI now.)

### 2. Fold into the projection (`src/projection.rs`)

Add a `policy` field to `Projection` (default `DecisionPolicy::defaults()`); in
`apply()`, handle `DecisionPolicyChanged` by `self.policy.set(class,
involvement)` (mirrors how `decisions` rebound). `resolve(class)` then reflects
the owner's configuration for that class.

```rust
pub struct Projection {
    // ...
    pub policy: crate::policy::DecisionPolicy,
}
```

### 3. Authority gate consults the real policy (`src/actions.rs`)

`actions::validate`'s `ProposeDecision` arm: check the claimed involvement
against `state.policy` (the projection-carried, event-derived policy) rather
than `DecisionPolicy::defaults()`. This is the enforcement fix — a configured
override to `Ask` now actually blocks a `Pm` claim.

```rust
PmAction::ProposeDecision { class, involvement, .. } =>
    policy::check_proposal(*class, *involvement, &state.policy)
        .map_err(PolicyError::DecisionPolicy),
```

### 4. PM resolves involvement from policy (`src/pm.rs`)

`plan_onboard` computes each proposal's `involvement` via the current policy
(`policy.resolve(class)`) instead of hardcoding. It must receive the policy
(from the `respond` caller, which already has the projection). This keeps the
PM's claim consistent with configuration so the gate (correctly) passes by
default but reflects overrides.

(Change `plan_onboard` signature to accept the current `&DecisionPolicy`, called
from `respond` with `&projection.policy`.)

### 5. Owner-facing endpoint (`src/web.rs`)

`POST /api/policy` `{ "class": DecisionClass, "involvement": OwnerInvolvement }`
→ appends `DecisionPolicyChanged` (actor = Owner) so the owner can configure
autonomy from the UI later; the event is the durable mechanism. Mirrors
`/api/decision` (owner action, direct append).

---

## File changes

| File | Change |
|---|---|
| `src/event.rs` | add `EventType::DecisionPolicyChanged` |
| `src/projection.rs` | `Projection.policy` field + fold `DecisionPolicyChanged` |
| `src/actions.rs` | `ProposeDecision` validate uses `&state.policy` |
| `src/pm.rs` | `plan_onboard` resolves involvement from policy; pass policy in |
| `src/web.rs` | `POST /api/policy` handler + route |
| Create `tests/decision_policy.rs` additions OR new `tests/policy_events.rs` | projection fold + gate enforcement tests |

---

## Step-by-step tasks (TDD, commit after each)

### Task 1 — Event type + projection fold
Add `EventType::DecisionPolicyChanged` (`event.rs`). Add `Projection.policy`
(default `DecisionPolicy::defaults()`) and fold it in `apply()`.

- Test (in `tests/decision_policy.rs` or a new `tests/policy_events.rs`): append
  a `DecisionPolicyChanged` (Database→Pm), build the projection, assert
  `proj.policy.resolve(Database) == Pm` and another class is unchanged.
- Run: `cargo test --test policy_events` — PASS.
- Commit: `feat(policy): fold DecisionPolicyChanged into projection (event-sourced)`

### Task 2 — Gate consults real policy
`actions::validate` `ProposeDecision` checks against `&state.policy`.

- Test: with a projection whose `policy` has Database→Pm, a `ProposeDecision`
  claiming `Pm` for Database passes; claiming nothing/less is handled; and
  (the key demonstration) a class owner-escalated to `Ask` rejects a `Pm` claim.
- Run: `cargo test` — PASS.
- Commit: `feat(policy): authority gate consults event-derived project policy`

### Task 3 — PM resolves involvement from policy
`plan_onboard` takes `&DecisionPolicy` and computes each proposal's
`involvement = policy.resolve(class)` instead of hardcoding.

- Test: onboarding still produces Database=Ask / testing-library=Pm under the
  default policy (existing vertical_slice tests keep passing); with an
  overridden policy the proposal reflects it.
- Run: `cargo test` — PASS; clippy/fmt clean.
- Commit: `feat(pm): derive proposal involvement from configured policy`

### Task 4 — Owner endpoint + live verification + docs
- `POST /api/policy` handler + route; verify via `cast run` (web boot test +
  manual): setting Database→Pm is durable (appears in `/api/state` policy scope
  or a follow-up projection), and the gate enforces it.
- Update `docs/HANDOFF.md` (roadmap item marked done; D6 note that policy is now
  event-sourced; add endpoint to the web section).
- Commit: `docs: policy is event-sourced + owner /api/policy endpoint`

---

## Tests / validation

- `tests/policy_events.rs` (new): projection fold of `DecisionPolicyChanged`;
  gate enforcement against an event-derived policy (override to Ask blocks a Pm
  claim); onboard involvement derived from an overridden policy.
- Full gate: `cargo test` (all green), `cargo clippy --all-targets -- -D warnings`
  (zero), `cargo fmt` (clean). Currently 61 tests.

---

## Risks / open questions

- **Backward/empty replay:** old logs with no `DecisionPolicyChanged` fold to
  defaults — safe, no migration.
- **PM claims must match configured policy:** if the owner escalates a class the
  PM was happily delegating, the PM's next proposal for that class would claim
  the old involvement and be **rejected** by the gate (correct, safe behavior) —
  but the scripted PM must derive from the current policy (Task 3) so it adapts
  rather than breaks. This is the intended semantic.
- **Who may change policy:** owner only for now. Future: agent-proposed policy
  changes routed through the same gate (YAGNI now).
