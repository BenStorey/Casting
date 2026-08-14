# PM wake≠act tiering + cache-write accounting

Date: 2026-08-14
Owner: Ben.
Status: Plan.

## Goal

Two deterministic, testable improvements to the D2 cost/economics model:

1. **PM wake≠act tiering (docs/PM_INVOCATION_TRIGGERS.md).** Implement the
   Tier-0/1/2 trigger model + quiet-window drain so the PM's expensive ACT path
   (git-observe + drain + respond + reconciler) is NOT run on every low-value
   event — it runs on a Tier-0/1 interrupt, or after a quiet window, bounding
   LLM spend on progress churn.
2. **Cache-write accounting.** `cache_creation_input_tokens` (cache writes, the
   ~10x component) is hardcoded `0` in the orchestrator AND advisor, so if a
   provider ever reports it we silently drop it, and `cache_hit_ratio` can't
   reflect write cost. Thread it through from provider usage so it's real.

## Priority note

The scripted PM path is already cheap (respond only ACTs on owner messages /
owner decisions). The real cost lever is the **live loop**: it currently runs
`observe_once + drain + respond + reconciler` on EVERY 500ms wake. With the LLM,
Tier-2 churn (CommitObserved, TaskCompleted, TestsPassed) waking the path is
wasted work. Tiering gates that. This is item 6 on the roadmap ("tiering per
event-type is not yet implemented").

## 1. Wake≠act tiering

New small module `src/wake.rs` (pure, fully unit-testable):

```
pub enum WakeTier { Immediate, Single, Batch }

pub fn tier_of(et: EventType) -> WakeTier   // the tier table (doc §Tier 0/1/2)

pub fn should_act(new_events: &[Event], quiet_elapsed: bool) -> bool {
    quiet_elapsed || new_events.iter().any(|e| tier_of(e.event_type) != WakeTier::Batch)
}
```

Tier table (from PM_INVOCATION_TRIGGERS.md):
- **Immediate (Tier-0):** MessageSent, DecisionMade, RequirementChanged,
  BudgetSet, WorkPaused/Resumed (the harness signals), IncidentDetected-ish.
- **Single (Tier-1):** TaskBlocked, ChangeSetReady, ReviewCompleted,
  WorktreeProvisioned/Removed, TaskReadyForReview.
- **Batch (Tier-2):** TaskCompleted, CommitObserved, TestsPassed,
  ObservationCreated (low severity), TaskCreated/Assigned/Started (progress).

`run_pm` loop change: on each 500ms wake, classify the newly-arrived event(s).
- If `should_act` (any Tier-0/1, OR the quiet window elapsed i.e. the poll
  timed out) → run observe + drain + reconciler as today.
- If only Tier-2 and no quiet window yet → defer (do not drain; cursor keeps
  accumulating; the poll timeout / a later Tier-0/1 event flushes).

This is exactly the doc's drain-flush conditions: (1) quiet window elapsed,
(2) a Tier-0/1 interrupt, (3) all idle. `drive_pm` (the test/CLI entry) keeps
calling `drain` directly — unchanged, so the deterministic tests are unaffected.

## 2. Cache-write accounting

- `client::Usage` already parses `prompt_tokens_details`; add the cache-write
  field if a provider reports it (OpenAI-compat doesn't today, but the field
  should be threaded, not hardcoded).
- Thread `cache_creation_input_tokens` from usage → orchestrator + advisor
  `CostMetering` (currently literal 0) → `CostIncurred` → `SpendView`.
- `cache_hit_ratio` math in `mental.rs` already excludes writes correctly
  (`reads/[prompt+reads+writes]`); with writes now real it stays correct.
- Honest framing: OpenRouter/OpenAI-compat don't currently report cache writes
  as a separate number, so it will usually land 0 — but if a provider does, we
  no longer drop it. Test the parse + the ratio math.

## Out of scope
- Actually invoking the LLM conditionally per tier (that's what the scripted
  respond already restricts; tiering here is about WHEN the path runs at all).
- Proactive advisor alerts / push.
- Multi-project (removed by design).

## Tests
- `wake`: tier classification for representative event types; `should_act`
  (only-batch defers, any-Tier0/1 acts, quiet-window acts).
- `drive_pm` unchanged still passes (regression guard).
- cache-write: a stub usage that reports a cache-write count flows into
  `CostIncurred` + `SpendView` + the ratio.
- Full `make` gate green.
