# Casting — PM Invocation Triggers

Status: Design note (day-1 "dumb" behavior, extensible later)
Purpose: Define WHEN the Project Manager runs, cheaply. Complements
ADDENDUM.md §1–§11 (the PM control loop). This doc is about the wake
side; the act side is already covered there.

## The core principle

Separate WAKE from ACT.

- WAKE = a cheap, dumb signal that "maybe go look". Costs ~nothing.
- ACT  = an expensive PM reasoning pass over everything new since its
         cursor, ending in "do X" or "nothing to do".

The expensive part is ACT. Batch and bound it. WAKE can (and should) be
reflexively simple — which is why "dumb" logic is the right day-1 choice.

Hard rule for day 1:

> NEVER invoke the LLM agent on every event.

A burst of events must collapse into a single PM pass (coalescing), not
one pass per event.

## Tiered trigger model

All triggers are cheap checks; none invoke an LLM by themselves.

### Tier 0 — immediate wake, always

Rare, high-value, must not wait:

- OwnerMessageReceived
- OwnerDecisionRecorded
- RequirementChanged
- IncidentDetected
- Budget threshold reached
- Security-critical event

Tier 0 interrupts whatever is happening. There are so few that batching
them would only add latency, never save meaningful cost.

### Tier 1 — wake on a single occurrence

Stalled or gated work — the project is waiting on PM action:

- TaskBlocked
- BuildFailed
- MergeConflictDetected
- AgentUnavailable
- DecisionRequested
- ChangeSetReady (where an approval gate exists)
- ReviewCompleted (when it unlocks downstream work)

One occurrence is enough to wake the PM.

### Tier 2 — batch, do NOT wake per-event

Normal progress:

- TaskCompleted
- CommitObserved
- TestsPassed
- low-severity ObservationCreated

These accumulate and only flush the PM under a DRAIN condition (below).

## Drain / coalescing

The cost lever for the PM specifically: process EVERY event since the
cursor in ONE wake. A burst of 40 CommitObserved events = one context
assembly + one strong-model call, not forty.

Flush the drain when ANY of:

1. a quiet-window elapses (e.g. 30–60s since the last relevant event), OR
2. a Tier-0 / Tier-1 interrupt arrives, OR
3. all active agents are idle.

On drain: assemble context once, run PM, let it emit structured actions
(or conclude "no-op"), advance cursor, sleep.

## Endorsed day-1 heuristics

### "Only check when all agents have finished work"

Good as a steady-state drain trigger, NOT the only trigger.
Strengths: never interrupts in-flight work; PM sees a coherent snapshot.
Weakness alone: too passive — the PM would sit dark during long async
work even when the owner just messaged, or when the whole team is stalled
behind one blocked task.

Conclusion: keep it as the "normalcy" flush, but keep Tier 0/1 as hard
interrupts that ignore agent idle-ness.

### "Consultants post that input is required"

Yes — and make it an EXPLICIT flag, not inferred. Give observations and
messages a structured field such as:

    pm_action_required: true | false

(or a severity where >= some threshold escalates). Escalation is then
DECLARED by the agent; the wake layer just checks the flag. The PM never
has to reason about whether a low-severity observation is worth its time.
Maps naturally to the "email" metaphor (respond vs FYI).

## What NOT to do

- Do NOT wake on a fixed timer ("every 60s, look"). Timers invite the PM
  to over-produce and burn money for no reason.
- Do NOT wake per LLM/tool telemetry (token streams, git plumbing,
  shell commands). That is runtime telemetry, not a PM concern.

## Day-1 concrete loop (~50 lines, no cleverness)

Wake condition: (new events since cursor >= 1)
  AND (quiet for N seconds OR all agents idle OR a Tier-0/1 event arrived)

On wake: assemble context once -> run PM -> structured actions or no-op
-> advance cursor -> sleep.

Every observation carries `pm_action_required` from day 1. Agents are
instructed to set it honestly; the wake layer trusts it.

## Later evolution

The wake rules can get smarter (relevance scoring, cost-aware batching,
per-agent importance) WITHOUT changing the act side at all — reinforcing
the addendum's wake/act boundary.
