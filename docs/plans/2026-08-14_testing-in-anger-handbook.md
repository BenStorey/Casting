# Casting — Testing-in-Anger Handbook (2026-08-14)

Goal: convert tomorrow's "test in anger" into an executable script with **pass
criteria and the exact knobs to watch**, so the session produces readable data
instead of a flailing hour. Based on the landmines review + the two new metrics
(owner engagement, code diff quality).

Everything you need to read is derived and exposed on the **Overview tab**
(`/api/model`) plus the **Activity stream** (`/api/events`) and the **Graph tab**
(`/api/graph`). Two small additions SSE'd in are the **owner engagement** and
**code diff quality** cards.

---

## The four meta-metrics (and where they live)

| Metric | Question it answers | Where to read it |
|---|---|---|
| **Cost / completed task** | "Is the PM burning tokens to say 'forward this'?" | `<Overview> Spend` card: `by_agent`, `total_estimated_usd`, `entries`. `Diagnostics` card: per-run prompt/completion tokens + est USD. |
| **Owner response rate** | "Am I being escalated to death / is the owner AWOL?" | `<Overview> Owner engagement` card: `awaiting_owner`, `response_rate`. |
| **Code diff quality** | "Is the code trending toward soup?" | `<Overview> Code diff quality` card: `avg_churn_per_commit`, `large_rewrites`, recent per-commit `+/-`. |
| **Recovery time** | "Does it boot/resume from a snapshot, not a 100k-event replay?" | `GET /api/health` (200 + latest_sequence) + observed boot time after restart. |

Rule of thumb: if those four look healthy, the architecture is real. When one
degrades, that's the landmine to chase (map below).

---

## Landmine → symptom map

| If you see… | Landmine (#) | Where it shows |
|---|---|---|
| Token cost flatlining while work stops / cost-per-task jumps | PM is the expensive bottleneck (1) | Spend, by_agent; OrchestrationRun tokens in Diagnostics |
| "It worked yesterday", model ignores tool schema, re-produces different code | Non-determinism (2) | Your job: confirm the PROJECTION replay stays identical (it must); code lives in git SHAs — diff the SHAs, not the projection |
| 5-min agent orchestration for a hex-code change, feels absurd | Trivial-task tax (3) | avg churn + elapsed time per trivial task |
| 5 consultants refactor one module 3 ways; PM keeps approving | Code ownership drift (4) | `large_rewrites` climbing; graph join points fanning out on one module |
| `awaiting_owner` grows while `response_rate` falls → owner muted | Owner abandonment (5) | Owner engagement card |
| Boot slows linearly with project age; snapshot not cutting the tail | Event-log / performance (6) | `/api/health` latency, boot time after restart |
| avg additions/commit rising + files-per-commit drifting; whole-file rewrite commits | Tool misuse / soup (7) | Code diff quality card + review-gate rejections |
| Cross-project file references / wrong secret in the wrong project | State bleed (8) | **Structurally impossible** — single-project, per-project secrets, worktree isolation |

---

## Scenario script

Run in order. Each: **FIRE → EXPECT → PASS → FAIL-LOOK**.

### 0. Baseline boot + recovery
- **FIRE**: `cast run /tmp/fresh` (fresh dir) or your project dir. Optionally
  `CAST_LLM_API_KEY=... CAST_LLM_MODEL=...` to enable the real LLM path.
- **EXPECT**: server binds quickly; `GET /api/health` → 200 + a `latest_sequence`.
- **PASS**: boots in flat time regardless of how much prior state exists
  (snapshot + tail, not a full fold from event 1).
- **FAIL-LOOK**: `/api/health` → 503 (store wedged — PG backend reconnect), or
  boot time scaling with event count (snapshot path broken).

### 1. Onboarding
- **FIRE**: send the objective as your first **owner message**
  (`POST /api/message` — wait for the 200, then sleep several seconds past the
  `step_delay` sequence before re-fetching).
- **EXPECT**: default cast hired (Marcus engineer, Maya QA), a plan forms,
  1-2 `OrchestrationRun`s in Diagnostics.
- **PASS**: `/api/model` shows the objective, ranked priorities, active agents;
  `spend` reflects metering (mock ~$0.0018/run when LLM off; real + non-zero
  when key set).
- **FAIL-LOOK**: partial state on first fetch (that's expected racing — refetch);
  empty plan; no `OrchestrationRun`.

### 2. Trivial task fast-path probe
- **FIRE**: ask for something tiny ("fix the typo on the login text", "change
  the button color to blue").
- **EXPECT**: it routes + completes without a production fan-out.
- **PASS (today)**: even if it walks the whole graph, it completes; note the
  elapsed time + spend delta. This is your **#3 baseline** for "is the fast
  path worth building."
- **FAIL-LOOK**: multi-minutes + several $0.0x for a one-liner → decide whether
  to add a trivial-task fast path (deferred item).

### 3. Task lifecycle + review gate
- **FIRE**: a normal task: it should go assign → start → complete → submit
  (`InReview`) → review → done.
- **EXPECT**: `TaskStatus::InReview` first; an approved `TaskReviewed` flips to
  `Done`. **A task must never reach Done without passing review.**
- **PASS**: Graph tab shows the state machine's legal transitions mirror the
  board; `Done` only post-approval.
- **FAIL-LOOK**: task skips `InReview` (gate bypass); rejected review bouncing
  back to Working loop (rework spiral — watch `large_rewrites`).

### 4. Hard-dependency ordering + Feature decomposition (opt-in)
- **FIRE**: with `CAST_DECOMPOSE=1`, send a cross-cutting feature request.
- **EXPECT**: PM creates a feature parent, fans out children
  (db/api/ui/sec), adds `BlockTaskOn api→db`, and every child runs the full
  lifecycle; the join resolves when all children are `Done`.
- **PASS**: `StartTask` gate **refuses** the hard-blocked child until its
  blocker is done (`PolicyError::BlockedByDependency`). Overlapping worktrees
  get distinct ports/branches.
- **FAIL-LOOK**: a child starts out of order; join resolves early; two children
  collide in one worktree/port.

### 5. Concurrent consultants / worktree isolation
- **FIRE**: two stakeholders into a decomposed feature so ≥2 consultants run
  in parallel.
- **EXPECT**: each gets its own worktree on `casting/task-<id>`, private
  `CARGO_TARGET_DIR`, distinct port (base 8081+).
- **PASS**: no cross-file bleed; `/api/model` `worktrees` lists them; reconciler
  prunes done/merged trees.
- **FAIL-LOOK**: shared target dir / port collision; file reverts; stale
  worktree left behind (StaleWorktreePass should clean up).

### 6. Budget halt
- **FIRE**: `POST /api/budget {limit_usd: <small>, warn_at: ...}`, then run work.
- **EXPECT**: spend climbs; GuardRail banner flips Warn → **Halt**; the LLM call
  is refused (guard is checked before the LLM call and every side effect).
- **PASS**: at Halt, `POST /api/pause`-equivalent resume does NOT un-halt (spend
  never decreases) — only a higher limit does. No spend happens after Halt.
- **FAIL-LOOK**: work proceeds after Halt (guard bypass in a side-effect path);
  Halt resumable by ResumeWork.

### 7. Pause / resume
- **FIRE**: `POST /api/pause`; send an owner message; wait; `POST /api/resume`.
- **EXPECT**: between pause/resume nothing acts (no new events from the PM);
  after resume, the queued work flushes.
- **PASS**: no events during pause; drain resumes after.
- **FAIL-LOOK**: the PM acts while paused (wake path bypasses `paused`).

### 8. Owner abstention → engagement metric
- **FIRE**: set a decision class to `Ask` (owner must decide); file a
  decision-requiring request; **don't respond.**
- **EXPECT**: `Owner engagement` card: `awaiting_owner` grows, `response_rate`
  falls below 1.0.
- **PASS (good sign)**: the backlog grows *and work is blocked on it* (no
  runaway); then respond and watch `response_rate` recover toward 1.0.
- **FAIL-LOOK**: PM silently auto-decides an `Ask` (autonomy leakage → #5 has
  lost its teeth); or `response_rate` is meaningless because it was never
  counted.

### 9. Diff-quality accumulation
- **FIRE**: deliberately run several features, then read the diff-quality card.
- **EXPECT**: `commit_count` grows, `avg_churn_per_commit` stable-ish, few
  `large_rewrites`.
- **PASS (today)**: you get a baseline you can trend across the week.
- **FAIL-LOOK**: `avg_churn_per_commit` climbing, `large_rewrites` spiking →
  consultants rewriting whole sections instead of editing surgically (#7) →
  this is the data-driven trigger to wire the configurable per-project
  `verify` gate (deferred).

### 10. Real LLM pass (if key set)
- **FIRE**: `CAST_LLM_API_KEY=... CAST_LLM_BASE_URL=https://openrouter.ai/api/v1
  CAST_LLM_MODEL=...`; run a real request.
- **EXPECT**: Diagnostics `OrchestrationRun` lists `metering_agent`, `provider`,
  `model`, real token counts, non-zero `estimated_usd`; `Spend`/by_agent moves.
- **PASS**: the model's `{"actions":[...]}` came back **gate-validated** — an
  unauthorized action is refused and *audited* as `PlanActionRejected`, never
  applied. Recovery-after-restart re-derives the same projection.
- **FAIL-LOOK**: `PlanActionRejected` count climbing from schema drift (model
  ignoring actions → everyday #2); spend with no `OrchestrationRun`; a refused
  action that still took effect.

### 11. Rejected-action visibility
- **FIRE**: coax the model (or script) into an illegal action (e.g. start a task
  with no worktree, or violate a transition).
- **EXPECT**: `/api/model` Diagnostics `rejection_count` increments; the
  `PlanActionRejected` row shows who/action/reason/correlation.
- **PASS**: you can see *why* the model was refused without guessing.
- **FAIL-LOOK**: a refusal logged only to stderr (the debugging-surface gap) —
  should be in the event log / stream.

### 12. Advisor + handoff
- **FIRE**: chat the Direction Advisor, then "Summarize and hand off to the PM".
- **EXPECT**: handoff becomes a **Briefing (`source: advisor`)**; the PM context
  (`/api/model` knowledge.briefings) shows it **marked advisory, not
  authoritative**.
- **PASS**: advisor context can inform but never sets rules; the private thread
  never polluted PM context before the explicit handoff.
- **FAIL-LOOK**: advisor advice surfaces as a directive/authoritative input.

### 13. External request intake + triage
- **FIRE**: `POST /api/request {source, reporter, title, body, labels}` (what a
  GitHub webhook will call), including a security-tagged one.
- **EXPECT**: deterministic triage → classification/severity/dup detection; shows
  in `/api/model` requests inbox; PM decides whether to act (policy-gated).
- **PASS**: a `security`/`bug` label triages as high/medium; duplicates collapse.
- **FAIL-LOOK**: a request triages as authoritative owner intent (it must stay a
  request with provenance).

### 14. Owner-as-consultant (human delivery)
- **FIRE**: have the owner take a task on directly and deliver via git.
- **EXPECT**: the observer records branch/commit/merge whoever pushes; the
  assoc->ChangeSet/provenance stays correct.
- **PASS**: task reaches Done through the normal review seam, sourced as the
  human's.
- **FAIL-LOOK**: a human commit bypasses the gate rails or loses provenance.

---

## If something fails first — triage order

1. **Is the store healthy?** `/api/health` (500/503 = backend/reconnect wedge —
   check `casts` daemon/PG, not the PM).
2. **What did the model actually see & do?** `Diagnostics` → `OrchestrationRun`
   context_summary + planned; `actor_contexts` shows each model's input. Never
   guess at prompt drift — read what was handed.
3. **Was it refused or authorized?** `rejection_count` / Activity
   `PlanActionRejected` distinguishes "model tried and was blocked" (fine) from
   "unauthorized thing took effect" (bug).
4. **Is it state or orchestration?** Rebuild the projection (`cast log --verify`).
   If the projection matches the event log but code looks wrong, it's **code**
   (git) not orchestration — diff the git SHAs, don't touch the projection.

---

## Prep already done (this session)

- **Owner engagement** card (awaiting_owner / owner_decided / delegated_decided
  / response_rate) — the #5 "are they engaging or muting" signal.
- **Code diff quality** card (churn aggregates + large-rewrite flag + recent
  per-commit) — the #7 soup signal, **language-agnostic** (git `--numstat`, no
  formatter assumption; captured into the event log at observe time).
- Both derived, read-only; the event log stays the only authority.

Deferred-by-design (don't build before you have week-1 data): configurable
per-project `verify` gate (the language-agnostic reformulation of
"fmt/clippy on every commit"), autonomous-actions daily digest, trivial-task
fast path, agent auto-memory. Each has a clear data trigger above.