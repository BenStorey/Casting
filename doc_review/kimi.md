Architecture.md-only Review (commit ef4cd25)

Structural / plan-level problems visible from the document itself, ordered by severity:

1. Budget guard ships opt-out (highest risk)
§16.1 shows Budget is event-set with no documented default limit, and §19's closing note names cost/liveness guard defaults as a review target — the doc's own tail admits the concern. Combined with §16.1's "Halted is permanent … spend never decreases", a first live run with an unset budget has no ceiling. The inert-default principle (§1) is applied to the orchestrator but not to spend: one enabled orchestrator + unset budget = unbounded burn. Improvement: document (and require) an explicit budget before any LlmOrchestrator is constructed — a startup precondition, not a runtime event.

2. Git semantic events are two-tier, and the hard case is unowned
§12.1 explicitly states the observer "Does NOT emit MergeConflictDetected" — that's deferred to "the git runner", but no runner emission path appears anywhere in §9/§12. So the conflict event exists in the vocabulary (§3.2) with no producer in the documented architecture. Either the runner path needs to be in the doc+plan, or the event is dead vocabulary.

3. Provenance/tracing is a floating module with no consumer story
provenance.rs and repo_metrics.rs are listed in §2 but never referenced in §5 (control loop), §15 (reconciler), or §19. /api/provenance/* endpoints appear with no documented producer cadence — provenance is only useful if captured at write time, and no event-flow shows that. Same for RepoMetricsCaptured (§3.2) — no documented trigger. These look like bolt-ons awaiting integration.

4. The path-safety boundary is documented for the repo but not for state/secrets
§13.3 covers resolve_under for the artifact repo. But §16.4 puts secrets.json inside .casting/ and §9.4 checks raw secret values — there's no documented guard preventing an agent activity from reading sibling state files (events.db, secrets.json) via Shell{cmd} with ../.casting/…. The hard invariant called out in §16.4 (raw secret in append-only log = forever) deserves a matching write-side boundary: document whether resolve_under also applies to Shell cwd/args, or shell is owner-only.

5. Single-writer concurrency story is implicit, not stated
§5.1 shows one run_pm loop and §4.2 relies on the IMMEDIATE txn for sequence safety, but nothing says "exactly one PM loop per binary" is an invariant, or what happens on a second cast run against the same .casting/ dir. WAL + immediate-txn makes collisions safe but not coordinated — two live PMs would double-act (both Tier-0 on the same owner MessageSent). Document a lock/ownership model (flock on events.db, or explicit "one runner per project") as a v1 invariant.

6. Dedup key excludes audit events by design — who watches retry storms?
§19.6 says audit/telemetry events are deliberately not deduped. With §5.1's multi-pass actor turns (max 10) and no documented per-cause retry cap, a crashing-after-schedule activity could accumulate duplicate ActivityScheduled/CostIncurred events on every drain. The idempotency protocol (§9.2 step 4) covers execution re-skip, but the event log itself grows unboundedly on crash loops. Consider a documented bound (max ActivityScheduled per id, or schedule-dedup on the activity id itself).

7. Cursor set vs. consumer set can drift
§4.4 names three consumers ("pm", "git-observer", "reconciler") but nothing enforces that set. The doc never says whether adding a fourth consumer (e.g. a future notifier) is a code change or a config entry — and CursorStore::advance is unconditional. Worth one line in §4: "consumer names are a closed set, code-defined."

Non-problems worth naming
- §14.1's StageManager "assignable per enum but excluded per default cast" tension is flagged in the doc itself — that's fine, it's self-aware. Just make sure is_assignable() and the gate agree, not just the cast list.
- §5.1's 500ms quiet window + Tier-2 batching is sound; the leapfrog and snapshot-race fixes (§19.7/19.8) are the right shape.

Where to focus improvement effort
1. Make budget a startup precondition for LLM mode, not an optional event.
2. Give MergeConflictDetected a real producer (or delete the variant).
3. State the single-PM-per-project invariant and enforce it (flock).
4. Extend the path-safety story to shell activities re: .casting/ siblings.
5. Wire provenance + repo_metrics into the documented event flow, or mark them explicitly deferred like D2.

№ 1, 3, 4 are the ones I'd block a "live LLM" milestone on.
