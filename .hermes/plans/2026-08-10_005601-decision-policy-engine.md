# Decision Policy Engine — Implementation Plan

> **For Hermes:** implement task-by-task with TDD; commit after every task.

**Goal:** Build a deterministic **decision policy engine** that encodes *delegated
authority* (brief §5): for each class of decision it resolves how much owner
involvement is required, so the PM knows whether to **ask the owner** or **decide
itself**. It is pure, LLM-free, fully testable, and sits directly in front of
the LLM seam (a future provider's plans flow through the same gate).

Importantly, the engine **does not decide whether to record a decision** — every
decision is recorded. It only decides **who the decision-maker is** (owner vs
PM) and **whether the owner's inbox is involved**.

**Architecture:** A new pure module `src/policy.rs` ("the engine") holds the
authority vocabulary and resolution logic. The PM loop (`src/pm.rs`) consults it.
**Every decision uses ONE universal event pair** — `DecisionProposed` →
`DecisionMade` — regardless of who decides. That pair is both the audit trail and
the seam: the only difference between "asked the CEO" and "PM handled it" is the
**actor** on the `DecisionMade` event.

**Tech Stack:** Rust (single binary, no new deps). serde for JSON round-trip.

---

## Current context / assumptions

- `PmAction::ProposeDecision` already exists but carries a free-form,
  never-enforced `owner_involvement: String` (only ever `"Required"`).
- `DecisionProposed` + `OwnerDecisionRecorded` exist today, but the pair is used
  only for the "ask the owner" path.
- There is **no** notion today of a decision the PM makes itself; every
  decision always blocks on the owner (delegated authority is unimplemented).
- **Representation decision (owner, 2026-08-10):** every decision — whether the
  owner answers it or the PM decides — is recorded with the SAME event pair.
  The pair is the universal audit record and the seam. The policy engine only
  picks *who decides* and *whether the inbox is involved*; it never suppresses
  a decision from the log. This keeps decisions auditable and non-noisy: a
  "decision" is a structured, recorded choice (options + recommendation +
  class), distinct from ordinary `MessageSent` chatter.
- `actions::validate` is the pure gate between any reasoning source and the
  event store; it currently treats `ProposeDecision` as pass-through.
- House style: curated enums (`EventType`), pure + unit-testable gates,
  `EventType` extended deliberately, clippy `-D warnings`, fmt clean,
  conventional commits, owner makes the big calls.

## Proposed approach / design

### 0. Universal decision lifecycle (the domain model)

Every decision follows exactly:

```text
DecisionProposed  (someone surfaces a structured choice: subject, options,
                   recommendation, class, involvement)
      │
      ▼
DecisionMade      (actor = the DECIDER: Owner if asked, or the PM/agent if
                   delegated; carries approved + optional note)
```

- The policy engine consults `DecisionProposed.class` → `involvement`.
- If involvement **requires the owner** (`Ask`) ⇒ the owner's inbox shows it and
  the PM **waits** for the owner's `DecisionMade`.
- Otherwise (`Never`/`Pm`/`Notify`) ⇒ the PM emits the `DecisionMade` itself
  immediately after proposing (actor = the deciding agent). No inbox noise, but
  the decision is still fully recorded.

This replaces the two divergent paths with one universal pair. `OwnerDecisionRecorded`
is retired in favor of the owner simply being one more **actor** on `DecisionMade`
(more uniform; the endpoint that records an owner verdict stays, it just writes a
`DecisionMade` with actor `Owner`).

### 1. The authority model (`src/policy.rs`)

Autonomy spectrum, least → most owner control (`Never < Pm < Notify < Ask`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OwnerInvolvement {
    Never,   // org may act; owner never involved
    Pm,      // PM may decide on its own
    Notify,  // owner informed, work proceeds
    Ask,     // owner must decide first (blocked until then)
}
```

A curated decision taxonomy (mirrors the brief §5 table):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionClass {
    InternalRename, InternalRefactor,   // Never
    TestingLibrary, AddConsultant, InternalImplementation, // Pm
    Database, Architecture, ProductRequirement,
    SpendingThreshold, ProductionDeployment, Irreversible,  // Ask
    SecurityCritical,                                         // Notify
}
pub fn builtin_involvement(class: DecisionClass) -> OwnerInvolvement { ... } // table above
```

A per-project policy with overrides + a safe default for unknown classes.

> **Owner config note:** the *builtin defaults are just seeds*. The owner will
> later configure per-class involvement from the UI (an autonomy knob per class,
> or "ask me about everything" ↩ "just build it"). So the exact default for any
> single class (e.g. `SecurityCritical`) is not load-bearing now — `DecisionPolicy`
> is designed to be overridden per class and (round two) persisted as events.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionPolicy {
    #[serde(default = "default_involvement")]
    pub default_involvement: OwnerInvolvement, // Ask = safe
    #[serde(default)]
    overrides: std::collections::HashMap<DecisionClass, OwnerInvolvement>,
}
impl DecisionPolicy {
    pub fn defaults() -> Self;                // builtin table, no overrides
    pub fn resolve(&self, class) -> OwnerInvolvement; // override → builtin → default
    pub fn set(&mut self, class, level);      // owner's configured autonomy
}
impl OwnerInvolvement {
    pub fn requires_owner_verdict(self) -> bool; // Ask → true; Pm/Never/Notify → false
    pub fn decider(self) -> Decider;             // Owner vs Pmt
}
```

### 2. Gate rule — no authority downgrade (`src/policy.rs` + `src/actions.rs`)

The seam-safety invariant: a producer may not claim *less* owner involvement
than the policy requires (an LLM/script error must not silently bypass the
human). Pure check:

```rust
pub fn check_proposal(class, claimed, policy) -> Result<(), PolicyError>;
// claim must be >= required (at least as restrictive) → else AuthorityDowngrade
```

### 3. Typed `ProposeDecision` + universal `DecisionMade` (`src/actions.rs`, `src/event.rs`)

- Replace the free `owner_involvement: String` with typed fields:
  `class: DecisionClass, involvement: OwnerInvolvement`.
- Add event type `DecisionMade`; retire `OwnerDecisionRecorded` (owner is just
  an actor on `DecisionMade`). `DecisionMade` carries `approved` (+ optional
  `note`) and takes `who`/actor = decider.

### 4. PM consults the engine (`src/pm.rs`)

Using the universal pair from §0, the PM's decision handling becomes uniform:

- `plan_onboard` builds the "Database choice" proposal through the engine
  (`DecisionClass::Database` → `Ask` → decider = **Owner**), so it stays in the
  owner inbox — but now it's policy-driven and typed.
- Demonstrate delegated authority with a second decision the PM owns:
  "Choose the automated-testing library" (`DecisionClass::TestingLibrary` →
  `Pm` → decider = **PM**). The PM emits `DecisionProposed`, then *immediately*
  emits `DecisionMade` (actor = PM) and creates the task — **no owner question,
  but the decision is still recorded**. (Real names/config tuned later; this
  proves both branches of the universal pair.)
- The existing `plan_owner_decision` (handles the owner's verdict on a proposed
  decision) now writes a `DecisionMade` with actor `Owner` instead of the retired
  `OwnerDecisionRecorded`.

### 5. Projection + inbox (`src/projection.rs`, `src/web.rs`)

- `Decision` gains `class: DecisionClass` + `involvement: OwnerInvolvement`
  (read from `DecisionProposed`) and a `decided_by` (owner vs agent) captured
  when `DecisionMade` arrives.
- Inbox item shows `involvement`/`class` so the owner sees *why* they're being
  asked (small, cheap). Only `Ask`-route proposals that are still `Proposed`
  appear.
- `/api/decision` (owner verdict endpoint) writes `DecisionMade` (actor `Owner`)
  instead of `OwnerDecisionRecorded`.

### 6. Registration (`src/lib.rs`)
`pub mod policy;`

---

## File changes

| File | Change |
|---|---|
| Create `src/policy.rs` | engine: `OwnerInvolvement`, `DecisionClass`, `builtin_involvement`, `DecisionPolicy`, `resolve`, `decider`/`requires_owner_verdict`, `check_proposal` |
| Modify `src/lib.rs` | `pub mod policy;` |
| Modify `src/event.rs` | add `EventType::DecisionMade`; retire `OwnerDecisionRecorded` |
| Modify `src/actions.rs` | typed `ProposeDecision` (class/involvement); new `PmAction::MakeDecision` (universal decider step); `PolicyError::AuthorityDowngrade`; `to_events` |
| Modify `src/pm.rs` | engine-driven `plan_onboard` (Database=Ask→owner) + TestingLibrary(Pm→PM); `plan_owner_decision` writes `DecisionMade` |
| Modify `src/projection.rs` | `Decision.class`/`.involvement`/`.decided_by`; handle `DecisionMade` |
| Modify `src/web.rs` | inbox item exposes class/involvement; `/api/decision` writes `DecisionMade` |
| Modify `src/main.rs` | none expected (verify) |
| Create `tests/decision_policy.rs` | pure engine tests |
| Modify `tests/policy_gate.rs` | update round-trip + add downgrade-rejection cases |
| Modify `tests/vertical_slice.rs` | update decision shape; add delegated-authority + universal-pair integration cases |

---

## Step-by-step tasks (TDD, commit after each)

### Task 1 — Authority vocabulary + defaults (engine core)
Create `src/policy.rs` with `OwnerInvolvement`, `DecisionClass`,
`builtin_involvement` (the §5 table), and `DecisionPolicy::defaults()`/`resolve`.
Register `pub mod policy;` in `src/lib.rs`.

- Test (`tests/decision_policy.rs`): `resolve` returns the builtin level for each
  class (Database→Ask, InternalRefactor→Never, TestingLibrary→Pm,
  SecurityCritical→Notify); unknown-class falls back to default (Ask); serde
  round-trip of the policy and enums; `PartialOrd`: `Never<Pm<Notify<Ask`.
- Run: `cargo test --test decision_policy` — expect PASS.
- Commit: `feat(policy): add authority vocabulary + default decision policy`

### Task 2 — Decider routing + downgrade gate
Add `OwnerInvolvement::requires_owner_verdict` + `decider`, and the pure
`check_proposal` (authority-downgrade rejection). Add
`PolicyError::AuthorityDowngrade` to `src/actions.rs`.

- Test: `decider(Ask)→Owner`, `decider(Pm)/decider(Never)/decider(Notify)→PM`;
  `requires_owner_verdict(Ask)→true`, others false; `check_proposal` accepts
  claim ≥ required, rejects a `<` claim with `AuthorityDowngrade`; `system`/
  owner may not be downgraded either.
- Run: `cargo test --test decision_policy` — PASS.
- Commit: `feat(policy): add decider routing + authority-downgrade gate`

### Task 3 — Typed ProposeDecision + universal DecisionMade
- `src/actions.rs`: `ProposeDecision` gains typed `class`/`involvement` (drop
  free string); add `PmAction::MakeDecision { decision_id, approved, note }`
  (the universal decider step, valid for owner OR a delegated agent).
- `src/event.rs`: add `EventType::DecisionMade`; remove `OwnerDecisionRecorded`.
- `Projection::apply`: read `class`/`involvement` off `DecisionProposed`; on
  `DecisionMade` set status + `decided_by` (from actor).
- Update `src/web.rs` inbox + `/api/decision` (writes `DecisionMade`, actor Owner)
  and any constructors in `tests/policy_gate.rs` / `tests/vertical_slice.rs`.

- Run: `cargo test` (all suites) — PASS. `cargo clippy --all-targets -- -D warnings` — 0. `cargo fmt`.
- Commit: `refactor(decision): typed policy + universal DecisionProposed/DecisionMade pair`

### Task 4 — PM consults the engine (delegated authority demo, universal pair)
In `src/pm.rs`: drive the Database proposal through `policy` (+
`check_proposal`) → decider Owner (in inbox). Add TestingLibrary → decider PM:
PM emits `DecisionProposed` then `MakeDecision` (actor PM) then creates the task
(no owner question, decision still recorded). `plan_owner_decision` writes
`DecisionMade` (actor Owner).

- Add integration tests in `tests/vertical_slice.rs`: (a) Database decision is
  **Proposed**, owner still decides, PM reacts to owner's `DecisionMade`; (b)
  TestingLibrary decision is **auto-resolved** — `DecisionProposed` **and**
  `DecisionMade` both present (actor PM), inbox has no TestingLibrary item, and
  a follow-up task was created — proving the universal pair end-to-end.
- Run: `cargo test` — PASS; `cargo clippy` — 0; `cargo fmt`.
- Commit: `feat(pm): consult decision policy — delegate Pm-level, ask owner for Ask-level`

### Task 5 — Verify whole product + docs
- `cargo build`; run `cast run` against the external workspace
  (`/home/ben/casting-workspace/`), confirm: Database decision in inbox,
  TestingLibrary handled autonomously, decision cards show class/involvement.
- Update `docs/HANDOFF.md` decision log + roadmap item #2 (decision policy
  engine) to **DONE**; note the engine is the delegated-authority seam.
- Commit: `docs: mark decision policy engine done (D2-build-around-seam item 2)`

---

## Tests / validation

- `tests/decision_policy.rs` (new, pure): builtin table, resolve defaults, serde
  round-trip, ordering, decider routing, downgrade rejections.
- `tests/policy_gate.rs` (updated): round-trip of typed `ProposeDecision` +
  `MakeDecision`; `AuthorityDowngrade` rejected; `MakeDecision` by the right
  decider accepted.
- `tests/vertical_slice.rs` (updated): universal pair — Database proposed→owner
  decides→PM reacts; TestingLibrary proposed→PM decides (both events present,
  actor PM, task created, no inbox). Also that the `/api/decision` owner verdict
  writes a `DecisionMade` (actor Owner).
- Full gate: `cargo test` (all green), `cargo clippy --all-targets -- -D warnings`
  (zero), `cargo fmt` (clean). Existing count is 47 tests, will grow.

---

## Risks, tradeoffs, open questions

- **Event-shape change:** `ProposeDecision`'s payload changes (free string →
  typed class/involvement) and `OwnerDecisionRecorded` is retired in favor of
  the universal `DecisionMade`. Existing stored dev state in
  `/home/ben/casting-workspace/state` still replays (old events default), and
  the scripted PM regenerates fresh decisions on a new run. Production-grade
  schema migration is out of scope (dev only).
- **Builtin defaults are seeds, not law:** because the owner configures
  per-class involvement later (autonomy knobs, round two), the default for any
  single class (e.g. `SecurityCritical` = `Notify`) is not load-bearing.
  `DecisionPolicy` is designed to be overridden per class and (round two)
  persisted as events so the owner controls the spectrum from the UI.
- **What counts as a "decision":** a decision is a *structured, recorded* choice
  (option + recommendation + class), distinct from ordinary `MessageSent`
  chatter, and is always captured by the universal pair. The curated
  `DecisionClass` taxonomy is the seed of "what's decision-worthy"; keep it
  deliberate (like `EventType`) so the log doesn't get noisy.
- **Delegated-decision audit trail: SOLVED by universal pair.** Even a
  `Pm`-level decision emits `DecisionProposed` → `DecisionMade` (actor PM), so
  auto-decided items are fully recorded — no separate code path, nothing lost.
- **`required >= claimed` comparison** uses the enum ordering (`Ask` is most
  restrictive). Ensure `Ord` on `OwnerInvolvement` is crystal-clear in docs.
- **Round two (not now):** persist `DecisionPolicy` changes as domain events so
  the owner can configure autonomy per class + a global "ask me everything" ↔
  "just build it" knob from the UI; add a dedicated `DecisionMade` audit view.
