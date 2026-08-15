# D2 LLM Wiring Audit

## Current state

The LLM infrastructure exists but **no real LLM call can happen today** because the API key from setup is never connected to the provider config.

## What's already built (no new code needed)

- [x] `LlmOrchestrator` — real LLM impl of the `Orchestrator` trait
- [x] `OpenAiClient` — OpenAI-compatible chat completions client
- [x] `ModelResolver` — per-actor model routing (consultants can declare model bindings)
- [x] `Advisor` — LLM-powered strategic advisor (separate from PM)
- [x] `parse_actions()` — model output → typed `PmAction`s
- [x] Policy gate — validates every proposed action (LLM or scripted, same gate)
- [x] Cost metering — token counts, `estimated_usd`, `CostIncurred` events
- [x] Guard rails — budget breaker, pause/resume, dispatch gate

## What needs wiring (the gap)

### 1. `from_env()` ignores the persisted API key  ← #1 blocker

`pipe_llm_orchestrator()` calls `llm::config::from_env()` which only checks `CAST_LLM_API_KEY`. The API key the user enters in the setup wizard goes to `RuntimeConfig.api_key` in `.casting/config.json` — but nothing reads it. So even a fully-configured setup never enables the LLM.

**Fix:** Make `from_env()` also check the state dir config. It needs the state dir path, which is available in `AppState.state_dir`.

### 2. No default model when env `CAST_LLM_MODEL` is unset

`from_env()` requires `CAST_LLM_MODEL` when `CAST_LLM_API_KEY` is set. The setup wizard doesn't ask for a model name, and users shouldn't need to set it. We should have a sensible default for OpenRouter (e.g. `deepseek/deepseek-v4-flash-0731`).

### 3. `pipe_llm_orchestrator()` doesn't receive the state dir

```rust
// control.rs line 246
pub fn pipe_llm_orchestrator(self) -> Self {
    match crate::llm::config::from_env() {  // ← no state dir passed
```

The `AppState` has `state_dir: Option<PathBuf>`. This needs to be passed through to `from_env()` so it can read `RuntimeConfig`.

### 4. Advisor has the same problem

`advisor_reply()` in `src/llm/advisor.rs` uses the `ModelResolver` which derives its base config from env only. No state dir fallback.

## Implementation steps

### Step 1: Make `from_env()` fall back to persisted config

Extend `from_env()` to optionally accept a state dir:

```rust
pub fn from_env(state_dir: Option<&Path>) -> Result<Option<ProviderConfig>> {
    // 1. Try env var first
    let api_key = match std::env::var("CAST_LLM_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            // 2. Fall back to persisted config
            match state_dir.and_then(|d| read_config(d)) {
                Some(cfg) if cfg.api_key.is_some() => {
                    // Use persisted key + defaults for model/provider
                }
                _ => return Ok(None),  // no LLM wiring at all
            }
        }
    };
```

Set sensible defaults: provider=`openrouter`, model=`deepseek/deepseek-v4-flash-0731`, base_url=`https://openrouter.ai/api/v1`.

### Step 2: Pass state dir through `pipe_llm_orchestrator()`

The `AppState` already has `state_dir: Option<PathBuf>`. Pass it to the updated `from_env()`.

### Step 3: Wire the advisor the same way

The advisor handler (`web/routes/advisor.rs`) builds the `ModelResolver` from `from_env()`. Pass the state dir there too.

## What stays scripted (intentionally not LLM-driven)

- **Budget guards, pause/resume** — deterministic policy, not a model decision
- **Decision policy** — the owner sets this, the PM doesn't decide its own authority
- **Policy gate validation** — pure deterministic check, never by an LLM
- **Task execution** (provision worktree, git operations) — platform actions, not model calls
- **Reconciler** (archive terminals, prune worktrees, opinion drift) — deterministic maintenance
- **Wake/tier logic** — deterministic classification of event urgency

## Test scenario

1. Purge → Start All
2. Complete setup wizard with a real OpenRouter API key
3. Send "Build me a todo app" in chat
4. First owner message triggers the LLM orchestrator path instead of the scripted path
5. PM calls OpenRouter → gets back typed `PmAction`s → gate validates → events appended
6. Cost metering visible in Debug view, budget breaker works