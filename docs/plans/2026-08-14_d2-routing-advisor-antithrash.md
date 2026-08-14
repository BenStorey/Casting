# D2 batch: per-actor routing, advisor LLM, reactive anti-thrash

Date: 2026-08-14
Owner: Ben.
Status: Plan.

## Goal

Three D2-thread items, done as one coherent batch:

1. **Per-consultant / per-actor model routing** — the consultant `ModelConfig`
   (provider/model_id/base_url/cost_tier/temperature/max_tokens) becomes real:
   different actors route to different models (Marcus→cheap, advisor→premium,
   someone→local LiteLLM) instead of everyone sharing the one env model.
2. **Direction Advisor LLM wiring** — Amara's replies stop being deterministic;
   the owner↔advisor thread gets real model replies. Low-volume, top-tier model
   (the exact economics the role was designed for).
3. **Reactive anti-thrash** — the PM stops re-proposing already-open decisions
   and supersedes stale ones, both via a deterministic gate AND the LLM's
   judgment.

## The unifying mechanism (1 & 3)

A **`ModelResolver`** maps an actor → resolved model binding + persona, using
the consultant registry + the env base config:

```
resolve(actor) -> ResolvedModel { config: ProviderConfig, system_prompt: String }
```

- Look up the consultant by **agent id** (`registry.by_id(actor)`, e.g.
  "marcus-reed"). If it declares a model binding (provider/model_id/base_url),
  build a `ProviderConfig` from it — **API key falls back to env** (keys stay
  out of consultant packages); base_url defaults through the provider map when
  the package omits it.
- If it also has a `system_prompt`, that's the persona.
- Fallbacks: actor "pm" → env config + PM persona; no consultant / no binding →
  env config + the actor's own persona.

`LlmOrchestrator` now holds a `ModelResolver` and resolves **per call** by
`context.actor` (the cost of building a client is trivial). Metering records the
resolved provider/model — cost stays attributable per agent, and it now reflects
the ACTUAL model that ran.

`pipe_llm_orchestrator` (in `pm.rs`) builds the resolver from the env config +
the consultant registry it already carries, so `cast run` gets routing for free.

## Advisor wiring (#3)

The advisor's output is a **free-form reply**, not a `PmAction`, so it does NOT
go through the `Orchestrator`/gate path. It reuses the SAME `ModelResolver` +
`OpenAiClient` though — so the advisor gets its own model binding.

When an owner→advisor `AdvisorMessageSent` arrives and the LLM is configured:
- Build the advisor context: Amara's persona (via resolver, actor "advisor") +
  the isolated `advisor_thread` so far.
- Call the model (free-form chat, no actions envelope).
- Append an `AdvisorMessageSent` reply with `actor = Advisor` (the existing
  event/projection already supports a reply landing in the same thread).
- A failed/blocked call (guard, HTTP error) → audit an `OrchestrationRun`
  error, no reply, no panic — same resilience as the PM.

Advisor stays ISOLATED from the PM until an explicit handoff — unchanged; we
only make the replies real.

## Reactive anti-thrash (#2)

Two layers — deterministic gate + LLM judgment:

1. **Deterministic gate (gate, not LLM):** `PmAction::ProposeDecision` is
   rejected if an OPEN decision with the **same subject** already exists
   (`PolicyError::DecisionAlreadyOpen`). This is the hard, testable anti-thrash:
   the PM physically cannot accumulate duplicate open decisions. (Superseding a
   closed one is unchanged.)
2. **LLM judgment (prompt):** the planning instructions tell the model: if a
   decision on this subject is already open, do NOT re-propose it — supersede a
   STALE/SUPERSEDED one (`supersede_decision`) or leave it. This is the "when"
   the roadmap deferred to D2; the LLM now reasons over `open_decisions` (already
   in `AgentContext`).

## Changes

- **`src/llm/routing.rs`** (new): `ModelResolver` + `ResolvedModel`; resolves
  actor → (ProviderConfig, system_prompt) from a `ConsultantRegistry` + base
  `ProviderConfig`. `model_from_consultant` helper (env-key fallback).
- **`src/llm/orchestrator.rs`**: `LlmOrchestrator::with_resolver(...)`; `plan`
  resolves per `context.actor`, builds a per-call client, uses the resolved
  persona + planning instructions; metering uses the resolved model/provider.
- **`src/llm/mod.rs`**: export routing.
- **`src/pm.rs`**: `pipe_llm_orchestrator` builds the resolver (env config +
  `self.consultants`).
- **`src/llm/advisor.rs`** (new): `reply_to_advisor(state, thread)` — resolve
  the advisor model, chat over the isolated thread, append a reply event (or
  audit an error). Called from the web handler.
- **`src/web/routes/advisor.rs`**: on owner `AdvisorMessageSent`, when LLM
  configured, generate the reply.
- **`src/actions/policy.rs`**: `ProposeDecision` duplicate-open-subject gate.
- **`src/llm/orchestrator.rs`** prompt: anti-thrash instruction block.
- **`src/guard.rs`**: (already the shared `llm_dispatch_allowed` gate — the
  advisor path consults it before calling, same as the PM.)

## Tests (stub-based, CI-safe, no live key)

- **Routing**: a consultant with `model_id`/`provider`/`base_url` resolves to
  that config (key falls back to env); actor without binding → env config;
  unknown actor → env config + fallback persona.
- **Orchestrator routing**: stub asserts the resolved model is sent (a
  per-actor model in the `body["model"]`) — proves the routing actually flows
  into the request.
- **Advisor**: owner message → stub returns a reply → an `AdvisorMessageSent`
  reply event (actor Advisor) is in the thread; advisor stays isolated (no PM
  context pollution); blocked/malformed → audited error, no panic.
- **Anti-thrash gate**: proposing a decision with an already-open subject is
  rejected; a different subject passes; superseding a stale one still works.
- **Prompt**: anti-thrash instruction present; `open_decisions` mentioned.
- Keep `make` green; ~286 → ~300 tests. Opt-in live test extended (optional).

## Out of scope
- Per-consultant TEMPERATURE/max_tokens wiring into the request (resolver
  carries them; passing them to the model is a one-liner follow-up).
- Advisor multi-turn memory beyond the thread (the thread IS the memory).
- Cloud/registry distribution of model bindings.