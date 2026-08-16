I've read the entire ARCHITECTURE.md. Here's my analysis of concerns, organized by severity.



Problems & Risks Visible from the Architecture Document

🔴 Structural vulnerabilities

1. Cursor-recovery audit-event inflation (§19.6, §5.4)

Audit events (OrchestrationRun, PlanActionRejected, CostIncurred) are deliberately excluded from the idempotency dedup set. This means every crash-recovery cycle re-appends them. Over time, a flaky PM loop will accumulate an unbounded number of duplicate audit entries, inflating the log and projections. The dedup key (event_type_debug, aggregate_id, correlation_id) also means two identically-structured actions in the same drain with different correlation_ids are treated as distinct — correct, but fragile if correlation_ids are ever reused.

2. LLM-call idempotency is semantic, not structural (§9.2)

The LlmCall{prompt} activity idempotency key prevents the side effect from re-executing, but a crash between ActivityScheduled and ActivityCompleted causes the same prompt to be re-sent to the LLM on restart — which may produce different output. The architecture has no mechanism to pin LLM output to a nonce or enforce deterministic replay. This is an acknowledged gap (the doc flags it implicitly) and the most likely source of "ghost" behaviors in production.

3. Port allocation race (elaborator vs. execution ordering) (§6.2, §19.8)

The worktree elaborator assigns free ports during planning, but ports are a finite pool consumed during execution. If two tasks in the same plan get two different worktree actions, they could each be assigned the same port on paper. The single-threaded PM loop probably serializes this in practice, but there's no explicit reservation+commit handshake described — just a "free port and slot from the worktree pool" which is a read, not a locked reservation. A future async extension (parallel worktrees, multiple concurrently-planning actors) would break this subtly.

4. Secret values can leak into human-facing fields (§16.4)

The ensure_no_raw_secrets guard only checks activities for raw secret values. Nothing scrubs MessageSent body, Briefing body, Decision content, or ExternalRequest fields — all of which are user/LLM-authored text that could accidentally contain a @secret:API_KEY@ value or paste the raw key. Once in the append-only log, that's permanent.

5. The Telegram poller is an unbounded concurrent writer (§17.2)

Telegram polling and web handlers and the git observer all write to the same event store concurrently. The PM loop has leapfrog protection but these concurrent writers can also interleave with the PM's read_since → build_projection → respond window, meaning the PM acts on a stale projection. The doc doesn't describe any optimistic concurrency or CAS append on the event store.



🟡 Design fragilities & missing seams

6. Default behaviour is invisible ("inert is the default") (§10, §10.3)

The system does nothing useful without an orchestrator. The MockOrchestrator only reacts to DecisionMade — it doesn't simulate PM initiative. A new user running cast init sees a system that appears to be dead, with no feedback about why nothing is happening. There's no startup-mode indicator that says "No orchestrator configured — system is inert."

7. Budget halt is permanent without escalation (§16.1)

Hitting the budget halts all LLM work. The only way out is to set a higher limit. There's no "increase time window" or "reset spend cycle" — just raising the cap. And spend never decreases, so ResumeWork is useless for budget halts. This is correct by design but the orthogonality of Pause/Resume vs. Budget is likely to confuse users.

8. ArchivePass is a one-way door (§15.2)

The reconciler removes terminal entities from the active projection to save context tokens. But there's no un-archive mechanism, no search-across-history, and no way to re-surface an archived fact or constraint. If the PM needs an archived opinion later, it's invisible. The event log has the data, but the PM acts only on the projection.

9. Add-a-variant hazard: PmAction validation has no compiler-enforced exhaustiveness (§19.3)

The document proudly flags "no wildcard arm" in validate(). But both validate() and to_events() are standalone match statements. Adding a new PmAction variant requires updating both manually — and the compiler will warn only if one of them uses non_exhaustive or has been recently compiled. A mismatch between the two is a silent runtime error, not a compile-time one.

10. Advisor isolation creates a knowledge silo (§10.4)

The advisor can't inject information into the PM's context except via an explicit AdvisorHandoff. If the owner tells the advisor something strategically important (e.g. "we're pivoting to microservices") but never triggers a handoff, the PM proceeds with stale context. The architecture treats this as intentional, but in practice owners will forget the handoff step.

11. SSE stream has no documented reconnection protocol (§11.1)

The /api/events/stream endpoint pipes the in-process broadcast to SSE clients. But what happens on reconnection? Can a client backfill missed events? The doc doesn't mention Last-Event-Id, cursor negotiation, or any recovery mechanism. Web clients will silently lose events during brief disconnections.

12. No structured observability (metrics, tracing, logging) anywhere

The entire architecture doc has no section on observability. The audit events are for audit, not operations. There's no way to measure PM loop latency, LLM call duration, side-effect duration, or detect a stuck loop. The liveness watchdog (§16 references) is mentioned as a pause trigger but its own design isn't described.



📌 Where to look hardest

The doc itself points you to the four areas to scrutinise in the closing paragraph. I agree, in this order:

1. Policy gate — the authority enforcement across all paths (not just PM-originated ones) needs auditing; the doc admits web handlers and git observer write concurrently.
2. Idempotency model — the dedup key, the audit-event exclusion, and the llm-call repeatability gap are the most likely failure modes.
3. Worktree elaborator — port allocation without a real reservation protocol will surface as a subtle race under load.
4. Cost/liveness guard defaults — disabled at startup means a production deployment could run unbounded spend before the owner even knows about the budget feature.

