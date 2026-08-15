# Casting — Autonomous Software Company

Casting is an **autonomous software company in a box**. You (the Owner) direct a team of AI specialists — a Project Manager, Engineers, QA, DevOps, and others — who plan, build, test, and ship software while you stay in the driver's seat.

You talk to the PM in plain language. The PM breaks your intent into tasks, assigns them to the right specialists, and escalates decisions to you only when needed. Everything is logged in an append-only event log — nothing is lost, nothing is guessed.

---

## Quick start

```bash
# Prerequisites
brew install rust   # or rustup.rs
cd frontend && npm install

# Init a project
cast init ~/my-project

# Run the workspace (API :8080 + Vite HMR :5000)
make dev
# Or: VS Code → Run and Debug → "Run API (cast run)" → then Run Frontend task

# Open http://localhost:5000 — the setup wizard walks you through it
```

---

## Architecture

### Single binary

Everything ships as ONE Rust binary (`cast`). The React SPA is compiled to static files and embedded into the binary via `rust-embed`. `cast run` serves both the JSON API and the SPA from one port (`:8080`). In development, Vite serves the SPA on `:5000` and proxies `/api` requests to `:8080`.

```
┌──────────────────────────────────────────────┐
│                  cast binary                   │
│                                               │
│  ┌──────────┐  ┌───────────┐  ┌────────────┐ │
│  │  CLI     │  │  Web API  │  │  PM Loop    │ │
│  │ (main.rs)│  │ (axum)    │  │ (control.rs)│ │
│  └──────────┘  └─────┬─────┘  └──────┬──────┘ │
│                      │               │         │
│              ┌───────▼───────────────▼───────┐ │
│              │      Event Store (SQLite)     │ │
│              │      + Projection             │ │
│              └───────────────────────────────┘ │
│              ┌───────────────────────────────┐ │
│              │   Embedded SPA (React/Vite)   │ │
│              └───────────────────────────────┘ │
└──────────────────────────────────────────────┘
```

### Event sourcing

All state is derived from an **append-only event log** stored in SQLite (default) or PostgreSQL. There is no mutable state — every change is a new event. The **projection** is rebuilt in memory by folding events in order. This makes the system auditable, debuggable, and crash-recoverable by design.

| Concept | What it is |
|---------|-----------|
| **Event** | An immutable record of something that happened (`TaskCreated`, `AgentHired`, `MessageSent`, etc.) |
| **Event store** | SQLite DB (or Postgres) storing events in sequence order |
| **Projection** | In-memory derived state (agents, tasks, decisions, budget, etc.) rebuilt from events |
| **Cursor** | Per-consumer position in the event stream (for PM, reconciler, Telegram, etc.) |
| **Snapshot** | Cached projection at a sequence number — optimization, never authoritative |

### PM loop

The PM runs as a background async loop inside `cast run`. On each cycle it:

1. Reads new events since its last cursor position
2. Rebuilds the projection (up to date)
3. Plans what to do next (deterministic scripted policy — no LLM calls yet)
4. Proposes typed actions through the **policy gate**
5. Appends accepted actions as new events
6. Sleeps until the next trigger (new event or timer)

The policy gate validates every proposed action against the current projection and the owner's configured decision policy. Rejected actions are logged as `PlanActionRejected` events — visible in the UI and auditable.

### Decision policy

The owner configures how much autonomy the PM has per decision class. Each class has an `OwnerInvolvement` level:

| Level | Meaning |
|-------|---------|
| `Ask` | Owner must decide before work proceeds |
| `Notify` | Owner is informed, work proceeds |
| `Pm` | PM decides autonomously |
| `Never` | Not a decision-worthy event |

Classes like `Database`, `Architecture`, `SpendingThreshold` default to `Ask`. Classes like `InternalRefactor`, `TestingLibrary` default to `Pm`/`Never`. The setup wizard offers three presets: "Run everything past me" (all Ask), "Only high-impact changes by me" (balanced defaults), "Do everything autonomously" (all Pm/Never).

### Frontend

React 19 + TypeScript + Vite + Tailwind CSS + Zustand (state management). The SPA is embedded into the Rust binary for production, but runs via Vite HMR in development.

**Data flow:**
1. Initial page load: fetches `/api/state` (full projection), plus `/api/model`, `/api/graph`, `/api/inbox`
2. Real-time updates: SSE at `/api/events/stream` pushes new domain events
3. On each SSE event: frontend re-fetches only `/api/state` (the projection, ~2KB)
4. Tab-switch: lazily fetches the data that tab needs (model, graph, or inbox)
5. Full refresh (all 7 endpoints) runs once every 30 seconds to catch up stale data

### Storage

| Backend | Use case | How to select |
|---------|----------|---------------|
| **SQLite** (default) | Single-user, local, zero-infra | `cast run <dir>` (auto) |
| **PostgreSQL** | Multi-process / production | `CAST_DB=postgres://...` or `--db` flag |

---

## CLI reference

```text
cast init <project-dir>
  Create + configure a project. Interactive wizard in the browser;
  headless via --name, --owner-token, --cast, etc.

cast run <project-dir>
  Start the workspace: PM loop runs, web API serves on :8080,
  embedded SPA or Vite HMR frontend available.

cast purge <project-dir> [--force]
  Delete .casting/ state directory — full reset. Project keeps
  its git history; Casting forgets everything.

cast brief <project-dir> [--subject S] [--source SRC] [--title T] <file|->
  Import external content as an advisory briefing (advises, never
  sets rules).

cast request <project-dir> [--source SRC] [--reporter R] [--label L] <title>
  Receive an external request (e.g. GitHub issue) into the intake.

cast log --db <events.db> [--project <id>] [--verify]
  Dump or verify the raw event stream.

cast smoke <dir>
  Append sample events and replay them (harness/test tool).
```

---

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `CAST_ADDR` | `127.0.0.1:8080` | API bind address |
| `CAST_DB` | `sqlite` | Storage backend selector |
| `CAST_OWNER_TOKEN` | — | Owner auth bearer token |
| `RUST_LOG` | `info` | Log level (structured logging) |
| `CAST_LLM_API_KEY` | — | LLM provider API key (D2 wiring) |
| `CAST_LLM_PROVIDER` | `openrouter` | LLM provider name |
| `CAST_LLM_MODEL` | — | LLM model id |
| `CAST_LLM_BASE_URL` | — | LLM endpoint (auto-derived per provider) |
| `CAST_SELFHOST` | — | Enable self-hosting (Casting building Casting) |

---

## Project structure

```
src/
  store/          Persistence backends (EventStore, CursorStore, SQLite, Postgres)
  event/          Domain event types, replay, integrity checks
  types/          Shared domain data structures (Task, Decision, Opinion, etc.)
  projection/     Read-model projection rebuilt from the event stream
  pm/             PM control loop, planner, decision policy, budget guards,
                  reconciler (background maintenance), triage classifier
  runtime/        Agent execution (orchestrator, executor, mental model,
                  wake logic, directives, context builder, persona, channels)
  workspace/      Project setup, git integration, secrets, auth, role catalog,
                  provenance tracking, repo metrics
  actions/        Typed PmAction vocabulary + policy validation gate
  consultants/    Loadable team member packages (identity + prompt per role)
  llm/            LLM provider config + client (OpenAI-compatible, D2 seam)
  web/            Axum router + route handlers (state, setup, graph, etc.)
  main.rs         CLI entry point
  lib.rs          Crate root

frontend/
  src/            React SPA (App, SetupWizard, views, API client, store)
  public/avatars/ Cast member placeholder images (SVG)
```

---

## Current status

**What works today:**
- Full CRUD event sourcing with SQLite (Postgres optional)
- PM control loop with deterministic scripted planning
- Decision policy engine with per-class autonomy levels
- Setup wizard (name → experience → cast intro → project → policies → API key)
- React SPA with live chat, board, team view, graph, inbox, decisions
- SSE live updates (projection-only refetch on each event)
- Budget guards, pause/resume, worktree provisioning
- Git observer + provenance queries
- Telegram owner channel (BotFather integration)
- CLI: init, run, purge, brief, request, log, smoke

**What's next (D2):**
- Wire the real LLM orchestrator (currently the PM uses a scripted policy)
- The typed action vocabulary and policy gate are the seam — an LLM producer
  drops in behind the same `Orchestrator` trait that the mock uses

**What's mocked / not yet wired:**
- LLM calls (the orchestrator trait exists, a mock exists, but the real
  OpenAI-compatible client is not connected — requires `CAST_LLM_API_KEY`)
- The PM's planning is deterministic scripted logic in `planning.rs`

---

## VS Code development

The `.vscode/` directory has ready-to-use configs:

| Action | How |
|--------|-----|
| **Build Rust** | `Ctrl+Shift+B` (Build (Rust only)) |
| **Launch API (debug)** | `F5` — builds, frees ports :8080/:5000, starts with LLDB debugger. All `log::info!` and `println!` output goes to Debug Console. Breakpoints work. |
| **Run Frontend** | Tasks: "Run Frontend" — Vite HMR on :5000 |
| **Purge state** | Tasks: "Purge project" — `cast purge --force` |
| **Test** | Tasks: "Test" — `cargo test` |
| **Lint** | Tasks: "Lint (clippy)" |