# D2 — Wire the real LLM orchestrator (OpenRouter day-one, LiteLLM later)

Date: 2026-08-13
Status: Draft → implements the deferred D2 item.
Owner: Ben.

## Goal

Replace the `MockOrchestrator` with a real LLM that drives the PM loop, while keeping
the provider a **config switch, not a code change** — OpenRouter on day one, a local
LiteLLM (or any OpenAI-compatible backend) later.

## Protocol decision (the whole crux)

Target **`POST /v1/chat/completions`** — the OpenAI-compatible chat-completions surface.

- `chat/completions` is the *common denominator*: OpenRouter, LiteLLM, vLLM, Ollama,
  LM Studio, DeepSeek all speak it natively.
- The newer OpenAI `Responses` API (`/v1/responses`) is NOT portable to local runtimes:
  LiteLLM only supports it via a bridge back to chat/completions; vLLM/Ollama/LM Studio
  don't implement it natively.
- Therefore `chat/completions` maximizes the "swap to LiteLLM / local later" goal. The
  `v1` path segment is just a namespace, not the moving part.

Provider → base_url resolution:
- `openrouter` → `https://openrouter.ai/api/v1`
- `litellm` → `http://localhost:4000/v1` (configurable)

A provider is `(base_url, api_key, model_id)` over one protocol. `provider` stays a
free string in `CostMetering` / `ModelConfig` (it already is — keep it that way).

## What already exists (positioned for this)

- `trait Orchestrator { fn plan(&self, ctx, cause) -> PlanOutput }` — the D2 seam.
- `PlanOutput { actions: Vec<PlannedAction>, metering: Option<CostMetering> }`.
- `CostMetering` with provider/model/prices — works for OpenAI-compatible usage shapes.
- `AppState.orchestrator: Option<Arc<dyn Orchestrator>>` + `with_orchestrator()`, OFF by
  default; `pm::respond` routes owner messages through it when enabled.
- Harness guards (`guard::llm_dispatch_allowed`) gate dispatch before the call.
- Consultant `ModelConfig { provider, model_id, cost_tier, temperature, max_tokens }`.
- `AgentContext` (context assembler) — exactly the surface an LLM reads.

## Changes

1. **HTTP client** — add `reqwest` (rustls, json). New `src/llm/mod.rs`:
   `OpenAiClient` (chat/completions) returning
   `ChatCompletion { choices[0].message.content, usage }`.
2. **Provider config** — `src/llm/config.rs`: `ProviderConfig { base_url, api_key,
   model, provider_name }`, resolved from env (`CAST_LLM_BASE_URL`, `CAST_LLM_API_KEY`,
   `CAST_LLM_MODEL`, `CAST_LLM_PROVIDER`) with the resolver mapping provider→default
   base_url. Default model per consultant when unset.
3. **ModelConfig.base_url** — add `base_url: Option<String>` to consultant `ModelConfig`
   (serde default) so a package can pin its own endpoint (e.g. a consultant that must
   hit local LiteLLM). Provider resolver still supplies the fallback.
4. **Async trait** — `Orchestrator::plan` becomes `async` + returns `Result<PlanOutput,
   LlmError>`. Update `MockOrchestrator` + the two call sites in `pm.rs` (guard branch +
   the orchestrator branch). The scripted `plan_*` functions stay sync.
5. **`LlmOrchestrator`** — `src/llm/orchestrator.rs` (or in `orchestrator.rs`):
   - Build a system prompt from the consultant's `system_prompt` (+ a Casting-specific
     instruction block describing the `PmAction` vocabulary + JSON output contract).
   - Build the user message from the `AgentContext` (context assembler serialized).
   - Request JSON mode (respond in a `{ "actions": [...] }` envelope of `PmAction`
     serde shapes, tagged `{"action": "create_task", ...}`).
   - Parse → `Vec<PlannedAction>` (actor `pm` by default); each `PmAction` still flows
     through `actions::validate` in `pm::run_planned`, so an LLM can only do what's
     authorized.
   - Fill `CostMetering` from `usage` + prices.
   - On malformed response / HTTP error → return an `Err` → logged via
     `PlanActionRejected`-style diagnostics, no spend beyond the failed call.
6. **Wire into `cast run`** (`main.rs::do_run`): when `CAST_LLM_API_KEY` (or config) is
   present, build `LlmOrchestrator` and `.with_orchestrator(...)`. Deterministic scripted
   PM remains the default when unconfigured. Print the provider/model banner at boot.

## Prompt → action contract

The LLM returns JSON:
```json
{ "actions": [
    {"action": "create_task", "id": "task-1", "title": "...", "kind": "feature"}
] }
```
`PmAction` uses `#[serde(tag = "action", rename_all = "snake_case")]` so the wire shape
is stable and round-trippable. The system prompt enumerates the valid actions, their
fields, and the gate rules (don't start a task without a provisioned worktree, etc.).
Rejected actions become `PlanActionRejected` events — the diagnostics/guard surface
already exists to surface them.

## Testing (no live key)

- **Stub OpenAI server** (`tests/llm_e2e.rs`): a tiny `axum`/`hyper` listener on
  `127.0.0.1:0` returning a canned `ChatCompletion` (fixed content + usage). Configure
  `LlmOrchestrator` with `base_url = http://127.0.0.1:<port>/v1` → assert the full
  seam: prompt built, HTTP POST shaped correctly, response parsed → actions validate →
  `CostIncurred` event → `proj.spend`.
- **Malformed-response test**: stub returns bad JSON → orchestrator errors, no panics.
- **Parse unit tests**: round-trip `PmAction` JSON both directions.
- Keep everything green with CI on a public repo (no real key).

## Out of scope (deliberately)

- Provider-picker UI / multi-backend config UI — a config switch is enough for day one.
- Adopting the `Responses` API — would hurt LiteLLM/local portability.
- Tool-calling / function-calling loop — Casting's "tools" are the validated `PmAction`
  vocabulary; no external tool protocol needed.
