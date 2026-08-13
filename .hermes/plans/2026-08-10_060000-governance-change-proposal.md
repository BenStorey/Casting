# Propose Directive Change (Governance via Decision Pipeline) — Plan

> **For Hermes:** implement with TDD; commit when green.

**Goal:** A mechanism for the PM/agents to *request* a change to governance when
they see a problem — without violating owner-only authority over directives.

**Architecture:** reuse the existing **decision pipeline** (`DecisionProposed` →
owner `DecisionMade` → PM reacts). Governance is owner-only, so a change cannot
be authored by the PM directly; instead the PM **proposes a `GovernanceChange`
decision** (class → Ask → lands in the owner's inbox). On owner approval, the
applied directive change is authored **as the owner** — the owner's explicit
approval *is* the authority that writes governance.

## Design

### DecisionClass::GovernanceChange
- Add to policy.rs, builtin involvement = **Ask** (routed to owner).

### New action `PmAction::ProposeDirectiveChange`
```rust
ProposeDirectiveChange {
    id: String,          // decision id
    subject: String,     // e.g. "Change TDD directive"
    kind, statement, scope, strength,   // proposed directive content
    supersedes: Option<String>,         // a directive to replace
}
```
- `to_events` → `DecisionProposed` with class=GovernanceChange, involvement=Ask,
  and the proposed change encoded in `options` (so the PM can re-read it after
  approval).
- `validate`: proposing is allowed for anyone (it's a proposal, not a change);
  the Ask involvement routes it to the owner.

### Apply on approval (pm.rs `plan_owner_decision`)
When an owner-approved decision whose class is GovernanceChange:
- Look up the decision, read the proposed change from `options`.
- If it supersedes an existing directive, emit `ProjectDirectiveCreated` (new,
  supersedes=old) AND mark the old one superseded.
- **Author all of these as "owner"** (who="owner"), so `check_directive_owner`
  passes — the owner's approval is the authority.
- Also send the normal approved message + create the adopt task.

### Wiring
- `plan_owner_decision` needs the proposal: build the projection to read the
  decision by id (it currently ignores `state`).

## Files
`policy.rs` (class), `actions.rs` (action + validate + to_events),
`pm.rs` (apply-on-approval), `tests/directives.rs` (new tests).

## Tests
- Proposal emits a GovernanceChange DecisionProposed; gate accepts it.
- Full loop: PM proposes directive change → owner approves (DecisionMade) →
  drive_pm → directive created/superseded, authored owner, old marked
  superseded.
- An approved GovernanceChange with no supersedes creates a fresh directive.

Commit: `feat(directive): PM can propose governance changes via the decision pipeline`.
