# Harness Responsibilities — What We Own vs. What We Borrow

> Immutable architecture rationale (owner 2026-08-10). Casting is a harness:
> it wraps agent calls, models, tools, and humans into one production system.
> This doc maps the ten harness responsibilities to what Casting ALREADY has
> (by design), what is genuinely missing, and — the key question — what we roll
> ourselves vs. piggyback on. The core claim: **an event-sourced core IS most
> of a harness already**, and adopting a generic agent framework (LangGraph /
> CrewAI / etc.) for orchestration would fight our foundational bet.

## The framing that decides everything

> **Our event log + cursors + projections + policy gate ARE the harness's
> spine (checkpointing, memory, validation, tracing).** A generic framework
> like LangGraph re-implements these with ITS OWN authoritative state
> (checkpoints, memory) — which would create two sources of truth and break our
> "event log is the ONLY authority" principle. So:
>
> - **We roll our own**: orchestration, state, memory, tracing substrate, gate.
>   These are the product and our moat — and we mostly already have them.
> - **We piggyback**: commodity *sub-libraries* (schema validation, retry/backoff,
>   OTel spans, rate-limit queues) and the *API gateway's* transport guarantees
>   (OpenRouter already does HTTP retry + throttling). We borrow libraries and
>   ideas, never their runtime.

---

## The ten responsibilities, mapped

### 1. Fault isolation & error boundaries — MOSTLY OURS, EXTEND D2
- **Have:** the policy gate (`actions::validate`) is a hard error boundary: a
  malformed/invalid action from any agent is recorded and skipped, never crashes
  the PM or the cast. The loop is synchronous and each append is validated.
- **Missing (D2 seam):** timeouts on the orchestrator/provider call, retry with
  exponential backoff, and a circuit breaker if a provider/agent keeps failing.
  Add these to the `Orchestrator::plan` seam, not the core.
- **Borrow:** a retry/backoff lib for outbound calls.

### 2. Context window management — THIS IS OUR PRODUCT, DONE DETERMINISTICALLY
- **Have:** the Context Assembler (`context_for` → targeted per-actor context),
  `operating_model` (/api/model → curated picture), projections as summaries.
  LangGraph's "checkpointing + memory + compression" is a re-implementation of
  **event-sourced projection**. We never send "the whole log" — we derive the
  relevant summary. This is a core bet, not a borrowed feature.
- **Missing (D2):** LLM-grade *summarization* on top of the deterministic
  projection (the deterministic layer stays the authority; the LLM can read a
  summary of it).
- **Borrow:** nothing — the mechanism is ours.

### 3. Structured output validation — OURS, EXTEND D2
- **Have:** agents emit typed `PmAction` enums, not free text; the gate validates
  and rejects invalid ones (returns to the caller). This is schema validation
  with the schema being the action type.
- **Missing (D2):** on an invalid response, feed the validation error back to
  the agent and retry (once), with a stronger-model fallback. The seam exists.
- **Borrow:** a JSON-schema validator lib if/when we validate arbitrary provider
  output, not our structured actions.

### 4. Observability / distributed tracing — OUR EVENT LOG IS THE TRACE
- **Have:** every event carries `correlation_id` + `causation_id`, so the full
  PM → consultant → minion → git call chain is reconstructable from the log.
  `/api/provenance/*` answers "why does this X exist?". This IS distributed
  tracing, as durable events rather than ephemeral spans.
- **Missing:** latency/token *metrics* and live span export (OTel) for real-time
  monitoring. Add an OTel exporter that turns events into spans for dashboards.
- **Borrow:** the OTel protocol / a metrics backend for the *transport*; the
  semantic trace stays events.

### 5. Checkpointing & resumability — OURS BY CONSTRUCTION
- **Have:** drop the process mid-task and restart → each consumer (PM,
  reconciler) resumes from its cursor; projections rebuild from the log.
  This is the strongest suit of event-sourcing. LangGraph checkpointing is a
  reinvention of this.
- **Missing:** nothing structural. In-flight "am I mid-append" is handled by
  the cursor advancing only after successful processing.
- **Borrow:** nothing.

### 6. Cost attribution & token budgeting — MISSING (D2)
- **Have:** nothing yet (no real provider calls).
- **Need (design the seam NOW):** tag every orchestrator/provider call with
  `agent_id`, `task_id`, `model_tier`, and record token/cost metering. The
  PM's "budget concern" needs this data; make `Orchestrator` return metering
  alongside its actions so it lands in the event log.
- **Borrow:** a metering/usage schema from OpenRouter's response envelope.

### 7. Concurrency & locking — OUR GIT IS THE LOCK, DESIGN INTENTIONALLY
- **Have:** git is the coordination surface (ownership boundary guarantees a
  real repo; ChangeSets are the unit of agent output; `MergeConflictDetected`
  event exists). Two agents writing the same file is a *git* problem, surfaced
  as a conflict event — not a corruption.
- **Need:** deliberate branch/ChangeSet policy so agents work on disjoint
  branches and merge via the PM; avoid "two editors on src/main.rs."
- **Borrow:** nothing (git is already the shared substrate).

### 8. Human escalation — PART OF OURS, ADD STATE MACHINE
- **Have:** owner-only auth, `open_decisions`, `DecisionProposed → owner`, and
  the Inbox. Escalation exists and is governed.
- **Missing:** *timed* escalation state machine ("wait N for owner, then default
  to autonomous") — a governance/directive concern, deterministic.
- **Borrow:** nothing.

### 9. Tool sandbox boundaries — OURS (ownership boundary), EXTEND FOR AGENTS
- **Have:** `workspace.rs` ownership boundary (self-identity guard, git runner
  sandboxing, path sandboxing, D5). Agents don't have raw shell yet.
- **Need (when D2 enables tools):** rootless containers / limited shell /
  rate-limited tool calls so a rogue agent or prompt injection can't `rm -rf /`.
- **Borrow:** a container runtime / sandbox (bubblewrap, etc.) for tool
  execution — the *sandbox mechanism*, not the policy.

### 10. Backpressure & rate limiting — OURS (synchronous loop), ADD QUEUE
- **Have:** the PM loop is synchronous (one drain at a time, step_delay), which
  is a natural global throttle — it can't spawn 10 concurrent agents today.
- **Need:** when we parallelize, a request queue with per-provider throttling
  and priority routing.
- **Borrow:** a queue/throttle lib (or the gateway's own rate-limit) for the
  transport; the priority logic is ours.

---

## Summary table

| # | Responsibility | Status | Roll vs. Borrow |
|---|---|---|---|
| 1 | Fault isolation | Mostly ours; add D2 timeouts/breaker | Borrow retry lib |
| 2 | Context mgmt | OURS (deterministic projection) | Roll (product) |
| 3 | Output validation | OURS (typed actions + gate) | Borrow schema lib if needed |
| 4 | Tracing | OURS (events = trace) | Borrow OTel transport |
| 5 | Checkpointing | OURS (event log + cursors) | Roll (product) |
| 6 | Cost attribution | **Missing — design seam now** | Borrow metering schema |
| 7 | Concurrency | OURS (git + ChangeSets) | Roll (policy) |
| 8 | Human escalation | Partial; add timed state machine | Roll (governance) |
| 9 | Tool sandbox | OURS (ownership boundary); add agent sandbox | Borrow sandbox mechanism |
| 10 | Backpressure | OURS (sync loop); add queue on parallel | Borrow queue transport |

## The one-liner

Roll our own: orchestration, state, memory, tracing, validation (the event-sourced
core — it's our moat and mostly exists). Piggyback: commodity libraries
(retry, schema, OTel, queue, sandbox) and the API gateway's transport guarantees.
Never adopt a generic agent framework's ORCHESTRATION runtime.

## Immediate next action (while deterministic surface is still in play)
Design the **#6 cost-attribution seam** on `Orchestrator` (return metering with
actions, land it in the event log) so that when D2 wires real providers, spend
is attributable from day one. Everything else is either already ours or cleanly
deferred behind the D2 seam.