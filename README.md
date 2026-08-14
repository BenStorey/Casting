# Casting

An **autonomous software company in a box**: an event-sourced system where a
Project Manager (PM) plans work, specialist consultant agents pick up tasks in
isolated git worktrees, and a **Direction Advisor** gives the owner a private
second opinion — all coordinated by a Rust control loop with an optional LLM
brain.

- Single self-contained binary: `cast run <project-dir>` serves the API + a
  React SPA on one port.
- **Event-sourced**: every decision, task, risk, opinion, commit and dollar of
  spend is an immutable event. Git owns *artifact* truth; Casting owns
  *organizational* truth (why things exist).
- **Deterministic by default, LLM-optional**: with no model configured, the PM
  uses scripted plans. Set `CAST_LLM_API_KEY` and the PM + advisor reason with a
  real model.

> The full design lives in `docs/`. Start with `docs/CASTING_PROJECT_BRIEF.md`,
> then `docs/HANDOFF.md` (current state + what's next). Newcomer/operator setup
> is here.

---

## Quick start

```
cd /home/ben/casting
make dev          # build + run the API (:8080) + Vite HMR (:5173) on a scratch project
```

The whole quality gate is one command:

```
make              # fmt → clippy -D warnings → test (all suites) → build (embed SPA)
```

Named targets: `make run` (API + embedded SPA only), `make frontend`, `make test`,
`make lint`, `make deploy-dev` / `make restart` (the dev.benstorey.com services).

`make run`/`make dev` target a **single project dir** (`~/casting-workspace/proj`)
— Casting is single-project (multi-project was cut by design). Pass any dir:
`./target/debug/cast run <dir>`.

## LLM wiring (optional)

Without an LLM the PM is deterministic (scripted). To enable the real model
layer, set env vars when you run `cast run`:

| Variable | Meaning | Default |
|----------|---------|---------|
| `CAST_LLM_API_KEY` | API key for the provider | *(unset → no LLM)* |
| `CAST_LLM_MODEL` | Model id, e.g. `deepseek/deepseek-v4-flash-0731` | *(required with key)* |
| `CAST_LLM_PROVIDER` | `openrouter` (day one) or `litellm` (local) | `openrouter` |
| `CAST_LLM_BASE_URL` | Override the provider's endpoint | provider default |
| `CAST_OWNER_TOKEN` | Shared auth token for the owner API | *(setup)* |

**Provider = config, not code.** Casting speaks one protocol — OpenAI-compatible
`POST /v1/chat/completions` — so the provider is just a base URL + key + model.
OpenRouter day one:

```bash
CAST_LLM_API_KEY=sk-or-v1-... \
CAST_LLM_MODEL=deepseek/deepseek-v4-flash-0731 \
CAST_LLM_PROVIDER=openrouter \
./target/debug/cast run /home/ben/casting-workspace/proj
```

Switch to a **local LiteLLM** (for Ollama/vLLM/other behind one proxy) by changing
two values, no code:

```bash
CAST_LLM_API_KEY=anything \
CAST_LLM_PROVIDER=litellm CAST_LLM_BASE_URL=http://localhost:4000/v1 \
CAST_LLM_MODEL=my-model \
./target/debug/cast run /home/ben/casting-workspace/proj
```

> 🔑 **Keys never live in tracked files.** A consultant package (TOML) can declare
> `model.provider/base_url/model_id` for **per-actor routing** (e.g. advisor →
> premium, an engineer → cheap), but the API key always comes from the env base —
> never from the package.

You can see who runs on what (and what it costs) in the UI's **Overview → Model
routing** section, and via `GET /api/routing`. The **Advisor** tab shows whether
the LLM advisor is active and its spend.

### What the LLM can and can't do

Every action a model proposes is validated by the **policy gate** before it's
executed — the LLM can only do what it's authorized to (no assigning to
non-existent agents, no starting un-worktreed tasks, no exceeding the budget).
The **budget breaker** and **pause** guard refuse LLM dispatch before any spend
once limits are hit. Metering is real: per-1M-token prices by cost tier feed
`CostIncurred` → the `/api/model` spend view.

### Tests

All LLM tests run against a **local stub** `chat/completions` server
(`tests/llm_e2e.rs`, `tests/routing_advisor_antithrash.rs`) — no live key, no
spend, CI-safe. An opt-in live round-trip is `cargo test --test llm_e2e -- --ignored`
with `CAST_LLM_API_KEY` + `CAST_LLM_MODEL` set.

---

## Deeper docs (in `docs/`)

- **`CASTING_PROJECT_BRIEF.md`** — the product concept + control-loop design.
- **`HANDOFF.md`** — current state, decisions, and what's next (read this before
  large changes).
- **`SEMANTIC_EVENTS.md`** — the event vocabulary.
- **`PM_INVOCATION_TRIGGERS.md`** — when the PM wakes (WAKE ≠ ACT tiering;
  implemented in `src/wake.rs`).
- **`HARNESS.md`** — the infrastructure responsibilities incl. cost attribution.
- **`DEPLOYMENT.md`** — the dev.benstorey.com host, systemd, and Caddy auth.
- **`consultants.md`** — the consultant/cast model + example packages.
- **`plans/`** — dated design plans for each slice (D2 LLM wiring, tests, routing,
  wake≠act tiering, ...).
