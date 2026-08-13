# Consultants — what it is to be a member of the cast  (2026-08-13)

A **consultant** is the packaged form of a member of the cast: a self-contained,
shareable TOML file (plus a system prompt) that defines an identity bound to a
catalog **role**, the routing hints the PM reasons over, the model binding that
feeds D2 cost metering, and its verification expectations.

Role remains the capability atom (`src/cast.rs`); a consultant is role +
identity + model + prompt + routing, ready to be handed to a provider when the
D2 LLM wiring lands.

```
src/consultants/mod.rs     types: ConsultantConfig, RoutingConfig, ModelConfig,
                           VerificationConfig, CostTier, ConsultantRegistry
src/consultants/loader.rs  embedded + filesystem loaders, strict validation
cast/                      curated DEFAULT packages (embedded in the binary)
  <role>.toml              one package per catalog role
  prompts/<role>.md        the system prompts
tests/consultants.rs       loader / registry / validation tests
```

## The TOML schema

```toml
[consultant]
id            = "devon-carter"        # machine key = file name basis; stable for sharing
name          = "Devon Carter"        # display / persona
title         = "Security Engineer"   # defaults to the catalog role's title
role          = "security"            # MUST resolve to a ROLE_CATALOG id (capability atom)
avatar        = "/avatars/security.svg"
summary       = "..."                 # free-text strengths → fed to the PM's routing context
system_prompt = "prompts/devon.md"    # relative path; MUST exist (fail-closed)

[consultant.routing]
specializations  = ["security", "authentication", "secrets", ...]  # hints, not a rules engine
trigger_patterns = ["auth", "oauth", "cve", ...]                   # case-insensitive containment hints
auto_join        = false               # true = part of the default cast (== AgentHired default)

[consultant.model]                    # per-consultant model tier (feeds D2 CostMetering)
provider      = "openrouter"
model_id      = "..."                 # PLACEHOLDER until D2 wiring sets real ids
cost_tier     = "standard"            # budget | standard | premium
temperature   = 0.1
max_tokens    = 4096

[consultant.verification]
review_required = true                # output must pass the InReview gate before Done
```

## Defaults vs. user packages

- **Curated defaults are EMBEDDED** in the binary via rust-embed (the `cast/`
  dir), so a fresh `cast run` works with zero setup: Marcus (engineer) +
  Maya (qa) are `auto_join`; Devon (security) + Priya (devops) are summoned
  specialists. Built-in defaults must always be valid — `from_embedded()`
  fails loudly otherwise.
- **Users drop packages** into `<project>/.casting/consultants/` (the collocated,
  gitignored state dir). A new `id` *adds* a consultant; reusing an existing
  `id` *overrides* the default. This is the "config, not script" story, and
  because every package is self-contained + id-namespaced, these same files are
  what a future sharing/marketplace layer distributes. A malformed present file
  is surfaced (not silently dropped); a missing directory is a no-op.
- Registry is loaded via `AppState.consultants` (embedded by default;
  `with_consultants` replaces it). `cast run` overlays
  `.casting/consultants/` using `load_consultants` in `main.rs`.

## The registry API

- `by_id(id)`, `for_role(role_id)` — the bound consultant for a catalog role.
- `default_cast()` — all `auto_join` consultants (hired by default).
- `specialists_for(task)` — consultants whose hints match a task description,
  best-match first. **A starting signal for the PM's routing reasoning, never
  an enforcement layer.** The PM (today scripted, tomorrow an LLM) makes the
  routing judgment over `specializations` / `trigger_patterns` / `summary`.
- `GET /api/consultants` — read-only JSON of the whole registry.
- Validation (fail-closed): unknown role, empty id/name, missing system prompt,
  temperature outside `[0,2]` → rejected loudly.

## What a consultant deliberately does NOT carry

- **No tool allowlists / blocked paths / minions / token budgets.** Isolation
  is a platform property (per-consultant worktrees, private `CARGO_TARGET_DIR`,
  distinct ports). Agents act only through the validated `PmAction` vocabulary.
- **No summon pricing / marketplace tiering.** Cost flows through the existing
  `CostIncurred` → `/api/model` spend seam and the `BudgetSet` guard; a
  consultant's `cost_tier` is configuration that feeds that metering, not a new
  billing ledger.
- **Minions (sub-agents) and a sharing/marketplace are FUTURE work** — the
  package format already anticipates them (namespaced ids, self-contained
  files) but they are not built into the definition.

## Living seam for D2

The D2 orchestrator reads `state.consultants` to pick, for each hired agent:
the per-consultant `model` (provider/model/tier → `CostMetering`), the loaded
`system_prompt` (the setup prompt), and the routing hints that help the PM
assign tasks. The wiring is exactly the point of this slice — model + prompt +
routing are now first-class, loadable configuration, not hardcoded.