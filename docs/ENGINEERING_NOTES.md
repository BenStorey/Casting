# Casting — Engineering Notes & First Slice Scope

Status: companion notes to CASTING_PROJECT_BRIEF.md (do not replace it).
Date: 2026-08-08
Author: Hermes (initial assessment)

## tl;dr

The brief is strong and unusually well-scoped. The core architecture
(append-only event history -> projections -> UI, with agent cursors) is
correct. Two things deserve attention before/while building:

1. Build the agent-as-CONFIGURATION model first; treat the persona/CV
   ("Maya Patel") purely as a renderer of that config. Don't let the
   personality layer eat the budget or obscure the typed Agent model.
2. Scope what `cast run` magic can actually be self-contained vs what
   needs an API key / network / bundled frontend. No one should discover
   the hard dependency at 11pm on first run.

## Assessment highlights

Strong:
- Append-only event history as source of truth; projections as current
  state; "don't make the UI rebuild state from the whole log" (ch 9,14).
- Event anatomy: causation_id / correlation_id / agent_run_id (ch 11) is
  what makes the "why?" experience (ch 41) possible. Keep first-class.
- Domain events vs runtime telemetry separation (ch 12). Correct.
- Agent cursors over transitive messaging (ch 16,17): a notification is a
  hint to consume persisted events, not the event itself.
- Delegated authority / decision policy engine (ch 5). Genuine product
  insight, not a checkbox.
- Aggressively not building Kafka/K8s/Temporal/Jira yet (ch 43) and
  starting with a simulated company (ch 36). Highest-leverage decisions.
- Rust as a product/deployment call, not ideology (ch 26-27).

Risks / gaps flagged:
- Persona vs technical model tension (ch 2.2) is real and underspecified.
  Persona must be a pure renderer.
- "One executable / zero prereqs" (ch 26,29,31) quietly needs an LLM
  provider + network + a frontend. Scope the minimum-magic boundary.
- The PM loop is the product but its wake-and-act procedure is
  underspecified (ch 4 lists ~15 duties without a decision procedure).
  The PM is effectively a state machine over the event stream with a
  cursor. Need a concrete spec of when it wakes / what triggers replan /
  how it avoids thrashing.
- Budget/cost (ch 6) is cheap to CAPTURE at event one (token/model/agent
  are already in events) even though cost REASONING is late (#11).
  Capture early, reason late.
- Version control is quiet. Decide: does Casting drive a real git repo,
  or does the repo live inside Casting's history? Not a blocker for the
  simulated first slice, but it's a "software company" meme decision.

## Recommended first vertical slice (scope)

Port of ch 36, no real LLM required. A single Rust binary that:

1. Opens a SQLite DB (WAL mode) in a project dir (e.g. my-project/.casting/).
2. Appends domain events with a per-project monotonic sequence +
   causation/correlation ids. Minimal set: ProjectCreated, AgentHired,
   RequirementCreated, TaskCreated, TaskAssigned, TaskStarted,
   TaskCompleted, ObservationCreated, DecisionProposed, OwnerDecisionRecorded,
   MessageSent. (Keep telemetry OUT; only domain events.)
3. Maintains current-state projections (tasks, agents, messages, decisions)
   rebuilt on demand from the log (idempotent, fine for slice one; do not
   store projections that can drift).
4. Has a SIMULATED PM/Eng/QA driven by a small deterministic script or
   canned behavior (brief explicitly permits simulation) that turns
   "build me a todo app" into requirements -> tasks -> work -> observation
   -> decision.
5. Serves a minimal web page (owner <-> PM chat + task list + activity
   stream + decisions) rendered from projections; realtime can be a simple
   poll or SSE — do NOT reach for a message broker.
6. Persists an owner decision as a durable OwnerDecisionRecorded event
   and updates the projection.

Success test (from ch 46, trimmed): run `cast run`, see the workspace,
tell the PM what you want, watch tasks appear and move, make a decision,
reload, everything still there, and you can trace WHY current state
exists.

## Anti-goals for slice one

- No event broker, no Postgres, no Kubernetes, no container orchestration,
  no real LLM multi-agent swarm, no Telegram/WhatsApp, no persona/CV polish.
- No projection caching that can drift (projections are recomputable).

## Open decisions to settle before/at kickoff

- D1: Rust toolchain floor — RESOLVED 2026-08-08. Upgraded this box from
  rustc 1.73.0 to 1.97.1 (stable), with clippy/rustfmt/rust-src present.
  MSRV is unconstrained; any current SQLx / axum / tokio release is fine.
- D2: LLM boundary — does slice one ship with a scripted PM, or wire one
  real provider behind an env var from day one? (Recommend: scripted loop
  for the harness, leave a thin `Anthropic`/`OpenAI` client stub.)
  **RESOLVED 2026-08-10 — scripted, and real LLM wiring deliberately
  DEFERRED.** Product surface is built around the seam first; a provider
  plugs in later (see HANDOFF.md §5, D2).
- D3: Frontend approach — server-rendered HTML + tiny JS vs a proper
  SPA build step (the latter costs the "single binary / no build tools"
  promise). Recommend starting server-rendered.
- D4: Git/artifact model (see risk above). Pick one, note it as a decision.

## Suggested crawl order (crates stay inside ONE binary until proven)

1. event types + SQLite append-only store + sequence
2. projections (tasks/agents/messages/decisions)
3. simulated PM/Eng/QA loop + agent cursors
4. owner->PM message + decision record events
5. minimal web UI + realtime (poll/SSE)
6. `cast run` scaffold + .casting/ layout

Crate-split (casting-domain / -application / -infrastructure / -web /
-cli) only when a compile-time or ownership boundary demands it.
