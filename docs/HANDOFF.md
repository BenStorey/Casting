# Casting — Project Handoff

Status: Handoff from current session to next agent/session
Date: 2026-08-09
Author: Hermes (acting on behalf of owner, Ben)

This document tells an incoming agent everything it needs to know to pick
up Casting: what the product is, what exists today, how to build/test/run
it, what has been decided, and what to do next. **Read the docs listed
here — they are the authoritative design.** This file is the map, not the
design itself.

---

# 1. Quick orientation

> **Status update (2026-08-09, updated):** two slices are built plus the policy-gate
> seam (`src/actions.rs`), the **ownership boundary** (`src/workspace.rs`,
> D5 / `docs/OWNERSHIP_BOUNDARY.md`), the **policy-gate hardening**
> (assignee checks), the **SSE catch-up** + **JSON 404** web fixes, and the
> **complete Git slice** (4 increments: boot-time repo management, semantic Git
> events + observer, ChangeSet as a first-class concept, and provenance linking).
> `cast run` takes `--repo` + `--state-dir` and ensures a real git repo at
> startup; a git observer turns raw branches/commits/merges into semantic domain
events; the projection renders ChangeSets; and `/api/provenance/*` answers
"why does this code exist?". **89 tests** (12+6+11+10+5+12+10+6+4+5+5+3),
> clippy <0 warnings, fmt clean, slice suites run in ~0s. Read on.
>
> **2026-08-09 follow-up fix:** the provenance routes were committed with axum 0.7
> `:param` syntax while this project runs axum 0.8 — that panicked `cast run` at
> router build, and the suite missed it because no test constructed the web
> router. Fixed (`{param}`) and a new `tests/web_boot.rs` boots the router to
> prevent recurrence. Also: **the dev workspace must live OUTSIDE the source
> tree** (`/home/ben/casting-workspace/`, per D5) — the old in-tree `.dev/proj`
> command in this doc and the VS Code tasks is corrected; the D5 guard refuses
> any repo inside the embedded source root.
>
> **2026-08-10 direction:** the owner has decided to **defer real LLM wiring**
> (D2) and instead build the deterministic product surface *around* the LLM
> seam — auth/multi-project, the decision policy engine, cost capture, task
> review, persona rendering. The scripted PM + policy gate remain the base; a
> provider plugs in later as a thin client over a complete product. See §5.

Casting is an agent-orchestration platform for building software, framed
as an **"autonomous software company in a box."**

- A human owner describes what they want.
- A Project Manager (PM) agent organizes a team of specialist agents
  (consultants), coordinates their work, manages priorities/budget, and
  asks the owner only when genuinely needed.
- The owner experiences it like running a small software company, not
  operating chatbots.

The core differentiators are: orchestration, governance, durable decision
history, delegated authority, cost management, shared project state, and
explainability ("why does this code/decision exist?").

**Authoritative design docs (read these first):**
- `docs/CASTING_PROJECT_BRIEF.md` — the full vision, product tenets,
  architecture, event model, tech direction, success criteria.
- `docs/ADDENDUM.md` — PM control loop + Git/provenance design (this is
  the meat of how the PM operates and how Git relates to Casting).
- `docs/PM_INVOCATION_TRIGGERS.md` — WHEN the PM runs (wake vs act,
  tiered triggers, coalescing, cost rules).
- `docs/OWNERSHIP_BOUNDARY.md` — the state-vs-repo ownership model +
  self-identity guard + self-hosting (D5, prerequisite for the Git slice).
- `docs/INITIAL_PITCH.md` — the original rough idea (context/history).
- `docs/ENGINEERING_NOTES.md` — scoping notes + open decisions (D1–D5).

---

# 2. What exists today

Two slices are complete and committed, plus the policy-gate seam and the
ownership boundary (both built 2026-08-09). An introducing note sits at the top
of §1. They are:

1. **Headless core** (slice one) — typed domain events, SQLite append-only
   event store, durable cursors, `cast` CLI. LLM-free, fully tested.
2. **First vertical slice — a simulated software company** (brief §36):
   projections derived from the event log, a deterministic *scripted* PM
   control loop, an owner inbox, a realtime web API, and a React SPA
   **embedded into the same binary**. No real LLM anywhere yet; the
   architecture is proven end-to-end.
3. **The PM policy gate + typed action vocabulary** (`src/actions.rs`):
   the addendum §16 seam. The scripted PM plans `PmAction`s and executes
   them through `actions::validate`, which rejects anything that breaks a
   project invariant before it becomes an event. Proven by 8 unit tests + a
   JSON round-trip; this is exactly the seam a real provider will occupy.
4. **The ownership boundary** (`src/workspace.rs`, D5): the self-identity
   guard (refuses to operate on the repo that built the binary), a mandatory
   state-dir always separate from the artifact repo, path sandboxing
   (`resolve_under`), and one pinned Git runner. `cast run` now requires
   `--repo` + `--state-dir`. Proven by 7 tests; prerequisite for the Git
   slice.
5. **Policy-gate hardening** (`src/actions.rs`): `validate()` now takes the
   acting agent's id and enforces that only the task's assignee (or `system`)
   may Start/Complete/Block it. Adds `PolicyError::TaskUnassigned` and
   `NotAssignee`. Also fixed `plan_owner_decision` which started a task without
   assigning it first. 12 policy-gate tests (4 new).
6. **SSE catch-up + JSON 404** (`src/web.rs`, `frontend/src/api.ts`): the
   SSE stream now accepts `?after=N` and replays missed events from the store
   before switching to live broadcast; the frontend tracks the last sequence
   seen and passes it on reconnect. Unknown `/api/*` paths return a JSON 404
   instead of falling through to the SPA index.html.
7. **The Git slice** (addendum §28, §18–27) — **COMPLETE**, 4 increments:
   - **Boot-time repo management** (`Workspace::ensure_repo`): `cast run`
     ensures a real git repo exists at `--repo` (git-init if missing); the
     preflight banner shows the git init, a true HEAD, and the current branch.
   - **Semantic Git events + observer** (`src/git_observer.rs`): the event
     vocabulary gains `BranchCreated`, `CommitObserved`, `MergeCompleted`,
     `MergeConflictDetected`, `ChangeSetReady`. A polling observer with a
     durable cursor (same shape as the PM loop) turns raw branches/commits/
     merges into those events. Runs at boot and on each PM drain.
   - **ChangeSet as a first-class concept** (`src/projection.rs`): the unit of
     agent output (task + branch + commits, ADDENDUM §21–22). Auto-derived as
     `Open` when a task branch appears; transitions to `Ready`
     (`ChangeSetReady` event) and `Merged` (`MergeCompleted` event).
     `CommitObserved` appends its sha to the matching ChangeSet.
   - **Provenance linking** (`src/provenance.rs`): pure query functions that
     walk the event log to answer "why does this code exist?" — the chain
     `commit → changeSet → task → requirement → decision → owner intent`
     (ADDENDUM §24–25). Three API endpoints: `GET /api/provenance/commit/:sha`,
     `GET /api/provenance/task/:task_id`, and `GET /api/provenance/decision/:id`
     (decision audit: who proposed, class/involvement, who decided, why).

The product milestone (§46) is nearly met: run it, meet the PM, tell it
what you want, watch requirements/tasks/agents appear, make a decision,
see it recorded permanently, reload — everything persists (verified).

## Repository layout

```
/home/ben/casting/                  <- PROJECT ROOT (this is the git repo root)
├── Cargo.toml                      <- single binary; lib crate "casting" + bin "cast"; build.rs
├── build.rs                        <- guarantees frontend/dist exists before compile (see §3)
├── src/
│   ├── lib.rs                      <- module root
│   ├── event.rs                    <- typed domain events (Actor, EventType, Aggregate, Metadata)
│   ├── actions.rs                  <- PM action vocabulary (PmAction) + policy gate (ADDENDUM §16)
│   ├── store.rs                    <- EventStore trait (append, read_since, latest_sequence)
│   ├── sqlite_store.rs             <- SQLite impl (WAL, append-only, per-project sequences)
│   ├── cursor.rs                   <- durable per-consumer cursors
│   ├── projection.rs               <- current-state projections derived from the log (§2.1)
│   ├── plan.rs                     <- Priority + Project Plan view (derived current plan)
│   ├── snapshot.rs                 <- projection snapshots (pure read optimization, never authoritative)
│   ├── pm.rs                       <- simulated PM control loop + shared AppState (§2.2)
│   ├── web.rs                      <- axum server: JSON API, SSE, embedded SPA (§2.3)
│   ├── workspace.rs                <- ownership boundary: self-identity guard + git runner (D5)
│   ├── git_observer.rs             <- semantic Git events from raw repo state (Git inc 2)
│   ├── provenance.rs               <- "why does this code exist?" queries (Git inc 4)
│   └── main.rs                     <- `cast` CLI (init, smoke, run)
├── tests/
│   ├── event_store.rs              <- 6 integration tests (headless core)
│   ├── vertical_slice.rs           <- 3 integration tests (projection + PM loop)
│   ├── policy_gate.rs              <- 12 unit tests (PmAction validation + assignee checks + JSON)
│   ├── ownership_boundary.rs       <- 10 tests (self-identity guard, sandbox, state-dir, ensure_repo)
│   ├── git_observer.rs             <- 11 tests (semantic Git events, ChangeSet auto-derive, merge)
│   └── provenance.rs               <- 3 tests (provenance chain: commit → task → requirement → owner)
├── frontend/                       <- React + Vite + TypeScript SPA (§2.4)
│   ├── dist/                       <- npm build output; GITIGNORED (build.rs writes a placeholder)
│   ├── index.html, vite.config.ts, tsconfig.json, package.json
│   └── src/{main.tsx, App.tsx, api.ts, index.css}
├── docs/                           <- all design docs (see §1)
└── .vscode/tasks.json              <- build/test/lint/run tasks (see §3)
```

## 2.1 `projection.rs` — derived current state

Folds the whole event log into a queryable `Projection` on demand
(`Projection::build(store, project_id)`), idempotently, per request.
Fields: `agents`, `requirements`, `tasks` (with `TaskStatus` =
backlog/working/blocked/done), `decisions` (`DecisionStatus` =
proposed/approved/rejected, incl. options + owner verdict), `messages`,
`observations`. **Never stored, never authoritative** — recomputed from
events (brief §9/§14, handoff principle 3).

## 2.2 `pm.rs` — the simulated PM control loop

- `AppState` — shared runtime state: event store + cursor store + active
  project + a tokio `broadcast` channel + a configurable `step_delay` (the
  animation pause, zeroed in tests). `AppState::append` writes the event to
  SQLite, then broadcasts it (a wake hint, never the source of truth).
- `run_pm` — the loop task: waits on the broadcast (500ms timeout as a
  safety poll), then **drains everything since its durable cursor** in one
  pass (abstracted as `drive_pm`, also the test entry). `Wake ≠ act` and
  coalescing per docs/PM_INVOCATION_TRIGGERS.md; it never reasons per
  event. No per-event LLM calls — there is no LLM yet.
- **The PM plans, then acts through the policy gate.** The scripted policy
  (`plan_onboard` / `plan_acknowledge` / `plan_owner_decision`) returns a
  `Vec<PlannedAction>` (`(who, PmAction)` tuples) — the SAME typed
  `PmAction`s an LLM will later emit. `run_planned` feeds each through
  `actions::validate`, which rejects actions that violate project invariants
  (assign an unhired agent, act on a nonexistent task, re-hire, duplicate
  task id) and only then converts them to domain events and appends them,
  updating a *running* projection so later actions in the same plan validate
  against earlier effects. Invalid actions are logged and skipped. This is
  the addendum §16 seam (`reasoning → actions → validation → execution →
  events`) proven end-to-end before any LLM is wired in.
- Scripted policy (D2 = scripted first): on the owner's **first message**
  it onboards the company, hires Marcus (engineering) + Maya (QA), creates
  requirements/tasks, completes them, posts an informational
  `ObservationCreated` (the feedback loop), then proposes a
  `DecisionProposed` ("Database choice", an **Ask-class** decision) and asks
  the owner via a message. It also demonstrates **delegated authority**: the
  build's testing-library choice is a **Pm-class** decision, so the PM decides
  it itself via the universal `DecisionProposed` → `DecisionMade` pair (actor =
  PM) — no owner question, but fully recorded. Subsequent owner messages get
  an acknowledged reply; an owner-authored `DecisionMade` gets acknowledged and
  (if approved) drives a follow-up task. Each event carries
  `correlation_id`/`causation_id`/an agent-run id, so the "why?" chain is
  already recorded.
- Events are emitted one at a time with a `step_delay` pause (default
  ~220ms) so the UI animates (brief §35); the delay is configurable and set
  to zero by tests, so the vertical-slice suite runs in ~0s.

## 2.3 `web.rs` — API + realtime + embedded UI

axum router for a single project (project id is currently fixed at
`project-demo` in `main.rs`; see §5 for multi-project):

- `GET /api/state` — the current projection (drives the UI).
- `GET /api/events?after=N` — raw event slice (activity/catch-up).
- `GET /api/events/stream` — Server-Sent Events, pushes every appended
  event live (SSE built with `futures::stream::unfold` on the broadcast).
- `GET /api/inbox` — decisions awaiting the owner.
- `POST /api/message` `{body}` — owner → PM; persists a durable
  `MessageSent` and wakes the PM.
- `POST /api/decision` `{decision_id, subject, approved, note}` —
  persists `DecisionMade` (actor = Owner).
- `POST /api/policy` `{class, involvement}` — owner configures a decision
  class's owner-involvement; persists `DecisionPolicyChanged` (actor = Owner),
  the event-sourced autonomy config the gate enforces.
- SPA serving: `rust-embed` embeds `frontend/dist/`; unknown extensionless
  paths fall back to `index.html` for client-side routing.

## 2.4 `frontend/` — React + Vite + TypeScript SPA

Deliberate owner decision: **a real SPA build step, not server-rendered**
(D3 reversed — see §6; "we'll need all the tools at our disposal").
Views: Chat (owner ↔ PM), Board (kanban: backlog/working/blocked/done),
Team, Decisions, Inbox (badge with unread count), Activity. Realtime via
SSE — on every event the app refetches `/api/state` + `/api/inbox`.
TypeScript types in `frontend/src/api.ts` mirror the Rust projection.
Dark themed, single CSS file, no CSS framework.

## 2.5 `cast` CLI

- `cast init <dir>` — create `.casting/` with `events.db` + `cursors.db`.
- `cast smoke <dir>` — headless-core smoke test (append/replay/cursor).
- `cast run --repo <dir> --state-dir <path> [--selfhost]` — boot the
  workspace: enforces the ownership boundary (D5), opens/creates the stores,
  seeds the project (`ProjectCreated` + hire `pm`) if empty, spawns the PM
  loop, serves API + embedded UI at `http://127.0.0.1:8080` (`CAST_ADDR` env
  overrides). `--state-dir` is **required** and always separate from the repo;
  `--selfhost` operates on the Casting source itself (see
  `docs/OWNERSHIP_BOUNDARY.md`). No prior `cast init` needed.

Known wart: the scripted titles read "Design Build me a todo app" (the
owner's message gets spliced into task titles) — cosmetic, low priority.

---

# 3. Build, test, run

Environment: Rust **1.97.1** (stable, via rustup). Frontend: Node **22** +
npm **10** (needed only for frontend work).

**The whole quality gate is ONE command (`make`):** fmt → clippy
(`-D warnings`) → test (all suites) → build (rebuilds the real SPA and embeds
it). While the project is small we keep build/test/run/deploy together in this
single step; we'll split it apart only if it gets too slow (see the
`Makefile`).
Targets:

```
make          # full gate: fmt + lint + test + build (~14s)
make run      # build + start the whole workspace (API + embedded SPA) on :8080
make dev      # build + cast run (:8080) AND Vite HMR (:5173) in one shell
make test     # cargo test only
make lint     # clippy --all-targets -- -D warnings
make fmt      # cargo fmt
make frontend # npm run build (rebuild the real SPA into frontend/dist)
```

Under the hood (kept documented for clarity / CI):

```bash
cargo build          # embeds frontend/dist (real SPA) -> target/debug/cast
cargo test           # 12+6+11+10+5+12+10+6+4+5+5+3 = 89 tests (all pass; ~0s)
cargo clippy --all-targets -- -D warnings   # keep at zero
cargo fmt            # format (rustfmt)
```

**Important:** `cargo build` embeds `frontend/dist/` at compile time
(rust-embed). That folder is gitignored; `build.rs` auto-writes a
placeholder `index.html` if it's missing, so a fresh checkout always
compiles and `cast run` always serves *something*. To embed the REAL SPA,
the SPA must be built BEFORE `cargo build` — `make`/`make run` encode that
order so you never have to: `cd frontend && npm install` (once), then
`make`.

Run the whole product (single binary — this is the milestone UX):

```bash
make run   # -> http://127.0.0.1:8080
```
Open the URL, chat with the PM ("Build me a todo app"), watch the team
form and tasks move, decide on the database in the Inbox, reload — all
state persists in `/home/ben/casting-workspace/state/` (kept separate from
the artifact repo, per the ownership boundary D5 / `docs/OWNERSHIP_BOUNDARY.md`).

> **⚠️ Do not use an in-tree workspace** (e.g. `.dev/proj`). The D5
> self-identity guard refuses any repo inside the embedded source root
> (`/home/ben/casting`), so `cast run --repo .dev/proj ...` fails at boot
> unless you pass `--selfhost` (which is the wrong semantic — it records the
> run as building Casting). The artifact repo and state-dir must live outside
> the source tree, exactly as `/home/ben/casting-workspace/` is set up. See
> `docs/DEPLOYMENT.md` + `docs/OWNERSHIP_BOUNDARY.md`.

Frontend dev with hot reload (`make dev` runs both in one shell; Ctrl-C stops
both). Or manually, two terminals:

```
make run                     # terminal 1: API on :8080
cd frontend && npm run dev   # terminal 2: Vite on :5173, proxies /api -> :8080
```
Open `http://127.0.0.1:5173`. `CAST_PROXY` env overrides the proxy target.

VS Code (opened at `/home/ben/casting`): Terminal > Run Task. Tasks:
Build, Build (release), Test, Test (single, prompts for filter), Lint
(clippy), Format, Run cast smoke, **Build frontend (SPA)**,
**Run workspace (cast run)** (always builds Rust + SPA first, then
serves the whole app), **Dev: API server** (builds then runs `cast run`),
**Dev: Frontend (Vite HMR)**. Required extensions: rust-analyzer, CodeLLDB,
crates, Even Better TOML.

---

# 4. Key architectural decisions already made

1. **Append-only event history is the source of truth.** Projections/current
   state are derived and queryable, but never the source of truth. Do not
   make the UI reconstruct state from the whole log per request (brief §9).
2. **Domain events ≠ runtime telemetry.** Keep them separate; the PM reacts
   to meaningful events, not machinery traces (brief §12, addendum §3).
3. **Event sourcing applied pragmatically, not as dogma.** Current-state
   tables (tasks/agents/decisions/…) are fine; the events tell how we got
   there (brief §14).
4. **Agents/consumers use durable cursors, not transient messaging** — a
   notification is a hint to consume persisted events, never the source of
   truth (brief §16–17). Implemented in `pm.rs` (broadcast → drain from
   cursor).
5. **The PM is a control loop, not a chatbot**, with a durable cursor and a
   persistent plan (addendum §1–2, 8). Implemented (scripted) in `pm.rs`.
6. **Wake ≠ act.** Never invoke the LLM per event. Coalesce via Tier-0/1
   interrupts + Tier-2 drain (PM_INVOCATION_TRIGGERS.md). The current loop
   has the drain shape; tiering per event-type is not yet implemented.
7. **Git owns artifact truth; Casting owns organizational truth; the
   integration owns provenance** (addendum §30). "Git knows what code
   exists. Casting knows why it exists." **Not built yet** — next slice.
8. **Persona/CV layer is a pure presentation of the underlying agent
   CONFIGURATION.** Ship the technical model first; bolt personality on
   later. (UI currently shows initials + role only.)
9. **Frontend is a real SPA (React + Vite + TS), embedded into the binary**
   (D3, owner's call — see §6). `cast run` stays a single self-contained
   artifact; dev uses Vite HMR + `/api` proxy.
10. **Anti-goals for now:** no Kafka, K8s, Temporal, EventStoreDB, message
    brokers, Telegram/WhatsApp, agent marketplace, Jira compat, GitHub
    integration first. No huge dashboard. (brief §43).

---

# 5. Roadmap / what's next

The simulated vertical slice is DONE and verified. Next increments, in
order (aligns with brief §45 priorities):

### Next: build the product around the LLM seam — DEFER REAL LLM WIRING (D2, owner decision 2026-08-10)
**Owner decision: do NOT wire a real LLM yet.** Build as much of the product
surface as possible around the seam, and plug a provider in *later*. This
reverses the earlier "LLM wiring next" priority.

The seam is already done and proven: the typed `PmAction` vocabulary + policy
gate in `src/actions.rs`, driven by the scripted `plan_*` policy in `pm.rs`
and fed through `run_planned`. Everything downstream — the durable cursor, the
drain shape, the event store, provenance — is provider-agnostic and stays.
The goal is to keep the product 100% deterministic / LLM-free for as long as
possible, so that when OpenRouter (the agreed day-1 provider; Ben has a key)
is finally wired, it is a **thin client over a complete product**, not a live
model racing a half-built scaffold. The `--no-llm` / scripted mode remains the
permanent default; a real provider is an *addition*, never the base.

Next increments build the deterministic product surface around the seam.

> **2026-08-10 prioritization (owner):** the **event / decision / state core is
> the product** — "this is what we live or die by." Focus effort there first and
> make it as mature as possible. **Auth + multi-project and cost capture are
> deprioritized** (not urgent; revisit later).

1. ~~**Decision policy engine** (brief §5) — *delegated authority*: the autonomy
   spectrum / decision-class → owner-involvement policy map. Deterministic,
   and it sits directly in front of the LLM seam (when a provider arrives, it
   decides how much the model is allowed to do before asking).~~ **DONE
   2026-08-10** — `src/policy.rs` (authority vocabulary + `DecisionPolicy` +
   downgrade gate), typed `ProposeDecision`, and the universal
   `DecisionProposed` → `DecisionMade` pair (actor = decider). Scripted PM
   demonstrates both branches: Database (Ask → owner inbox) vs
   testing-library (Pm → PM decides, fully recorded). 61 tests.

### Next: mature the event/decision/state core (priority)

Concrete candidates, roughly in value order — all deterministic, LLM-free,
independently testable:

1. ~~**Persist `DecisionPolicy` as domain events.** Today the per-class autonomy
   map is rebuilt from `DecisionPolicy::defaults()` in-memory; the owner's
   overrides aren't durable. Make policy changes first-class events
   (e.g. `DecisionPolicyChanged`) so delegated authority is *part of the event
   log* — the source of truth — not a hardcoded default. This is the natural
   completion of the policy engine and unlocks owner-configured autonomy.~~
   **DONE 2026-08-10** — `DecisionPolicyChanged` event (owner-authored via
   `POST /api/policy`), folded into `Projection.policy`; the gate and PM now
   derive involvement from the event-sourced policy. Verified live: escalating
   a class to Ask stops the PM auto-deciding it.
2. ~~**Decision audit / provenance view.**~~ *(deferred; see 2' below)* — instead the
   **Project Plan + priority reducer** was prioritized as the next state-core piece
   (below).

2'. **Project Plan projection + priority reducer** — **DONE 2026-08-10**. Added
   the deterministic current-plan as derived state: `Priority` enum
   (Critical>High>Medium>Low), `TaskPriorityChanged` event (mutation) reduced to
   `Task.priority`, `PmAction::SetTaskPriority` through the gate, and a
   `Projection.plan()` view (objective + ranked priorities + open decisions)
   exposed on `/api/state` (`plan`). First dogfooding artifact: our own roadmap
   could become this state instead of `.md`.
3. **Decision lifecycle maturity / anti-thrash.** Handle open-decision edge
   cases deliberately: re-planning when a decision is blocked on the owner,
   superseded/re-opened decisions, and recording *why* a decision was made even
   when delegated (the `note`), so no decision is silent.
4. **Event-stream integrity + tooling.** Harden the append-only core:
   sequence-integrity guarantees, a replay/export command, and invariants
   ("no gap in sequence", "a DecisionMade always follows a DecisionProposed").
   Add a `cast` CLI surface for inspecting the raw event log — the foundation
   everything else builds on.

**State-core maturity steps 1–3 — DONE 2026-08-10** (source of truth is the
event log; projections are derived; snapshots are never a source of truth):

- **1. Decision audit / provenance view.** `provenance::for_decision` answers
  who proposed a decision, its class/involvement, status, who decided it, the
  owner's note, and the chain back to the initiating owner message. Exposed at
  `GET /api/provenance/decision/{id}`.
- **2. Semantic state objects (SEMANTIC_EVENTS §8).** First-class `Risk`
  (full lifecycle: RiskRaised → RiskUpdated open/materialized/resolved, via
  RaiseRisk/ResolveRisk through the gate) + `Assumption` + `Constraint`
  (record-only notes). Flattened into the projection and surfaced in the plan
  (`open_risks`). "Agents interpret, the system records."
- **3. Snapshots.** SQLite-backed `SnapshotStore`s (`snapshots.db`); the READ
  path builds from snapshot + tail and falls back to a full fold on
  missing/corrupt. `Projection` types now Deserialize/PartialEq. Purely
  computational — the event log stays the only authority.

Then, only after the core matures:
5. **Task `review` status** — add the review lifecycle to tasks / ChangeSets.
6. **Persona / CV rendering** (brief §2.2) — the friendly identity layer,
   kept as a *pure renderer* of the underlying agent configuration.
7. ~~**Auth + multi-project** (brief §2.1/§31) — owner login + per-project
   workspaces.~~ **DEPRIORITIZED** (not urgent).
8. ~~**Cost capture** (brief §6) — spend/budget/forecast.~~ **DEPRIORITIZED**
   (not urgent).

Design note: every one of these is LLM-free and independently testable, exactly
like the Git slice. Keep `EventType` a curated enum and extend it deliberately.


### Local Git (addendum §28, 18–27) — COMPLETE (4 increments built)
All four increments are built, tested, and committed. The Git slice is
fully deterministic (no LLM anywhere). Everything uses the pinned git runner
from `src/workspace.rs` and `resolve_under`.
**Ownership boundary prerequisite (D5 / `docs/OWNERSHIP_BOUNDARY.md`) is LANDED**
— `src/workspace.rs` provides the self-identity guard, path sandboxing, the
single pinned git runner (`Workspace::git_command()`), and the mandatory
separate state-dir, so the Git workflow can never accidentally target the
Casting source. Everything below uses that runner and `resolve_under`. No LLM
anywhere; this is fully deterministic.

Proceed one increment at a time, in this order:

1. **Boot-time repo management.** `cast run --repo` ensures a real git repo
   exists at that path (git-init if missing); preflight shows a true HEAD; this
   wires Git into the workspace at startup. Testable on its own.
2. **Semantic Git events + a minimal observer.** Add the domain vocabulary —
   `BranchCreated`, `CommitObserved`, `MergeConflictDetected`, `MergeCompleted`,
   `ChangeSetReady` — as `EventType` variants, and a watcher that turns raw
   refs/commits into those events via the event store (durable cursor, same
   shape as the PM loop). This makes Git a first-class external system.
3. **`ChangeSet` as a first-class concept.** The unit of agent output: which
   task, branch, and commits produced a batch of work (ADDENDUM §21–22).
4. **Provenance linking.** commit → changeSet → task → decision → requirement →
   owner intent, so the UI can answer "why does this code exist?"
   (ADDENDUM §24–25).

Design note for the incoming agent: Git has more surface than prior slices —
draft the semantic-event vocabulary from ADDENDUM §18–30 and confirm with the
owner before locking it in; the owner makes the big calls. Git drives the
workflow; Git owns artifacts; Casting owns the organization.

### Meanwhile / opportunistic (small wins; fold in as fits)
- ~~**Policy-gate hardening**: `StartTask`/`CompleteTask`/`BlockTask` should also
  check the acting agent is the task's *assignee* (`src/actions.rs`) — hardens
  the LLM seam.~~ **DONE** (12 policy-gate tests).
- ~~**`/api/*` JSON 404 wart**: unknown API paths fall through to the SPA
  index.html; return a JSON 404 instead.~~ **DONE**.
- ~~**SSE catch-up**: stream only pushes new events; replay missed on
  reconnect.~~ **DONE** (`?after=N` + frontend tracking).
- **Auth + multi-project** (brief §2.1/§31, first-run UX), **task `review`
  status**, **cost capture** (metadata shape is ready) — larger; leave to their
  own milestone.

### Later: Postgres, decision policy engine, realtime dashboard polish,
external owner messaging (Telegram/WhatsApp), context-assembly scoring,
agent identity/persona rendering.

---

# 6. Decision log (D1–D6)

- **D1 — Rust toolchain floor: RESOLVED.** 1.97.1 on this box.
- **D2 — LLM boundary: SCRIPTED, and REAL LLM WIRING DEFERRED (owner decision 2026-08-10).** The
  vertical slice ships a deterministic scripted PM (no provider calls). The
  `PmAction` vocabulary + policy gate in `actions.rs` (built 2026-08-09)
  are the addendum §16 seam, proven by tests. **Day-1 provider (when wired):
  OpenRouter** (Ben's call; he has a key). As of 2026-08-10 the owner has
  decided to **defer LLM wiring** and instead build the deterministic product
  surface around the seam (auth/multi-project, decision policy engine, cost
  capture, task review, persona rendering — see §5). The provider, when it
  arrives, is a thin client over a complete product — never the base.
- **D3 — Frontend approach: RESOLVED — real SPA.** Owner chose React +
  Vite + TypeScript over server-rendered HTML, accepting the build step;
  the SPA is embedded into the binary to preserve the single-artifact
  deployment story. Vite HMR + `/api` proxy is the dev workflow.
- **D4 — Git/artifact model: RESOLVED — local Git first.** Settled in
  ADDENDUM.md; do not build GitHub before local Git.
- **D5 — Ownership boundary: RESOLVED — build-time self-identity guard.**
  Casting refuses to operate on the repo that built it (embedded source root +
  `name = "casting"` identity in `build.rs`); all Git runs through a single
  pinned git interface; the **state-dir is mandatory and always separate** from
  the artifact repo (no collocated default); **self-hosting** (Casting building
  Casting) is an explicit `CAST_SELFHOST=1` opt-in that records the build
  commit. Full rules in **docs/OWNERSHIP_BOUNDARY.md** (added 2026-08-09,
  prerequisite for the Git slice).
- **D6 — Decision representation: RESOLVED — universal decision pair + policy
  engine (owner decision 2026-08-10).** Every decision — whether the owner
  answers it or a PM/agent decides — is recorded with the SAME event pair:
  `DecisionProposed` → `DecisionMade`; the only difference is the **actor** on
  `DecisionMade` (Owner vs the delegated agent). `OwnerDecisionRecorded` is
  retired. The new `src/policy.rs` decision-policy engine (brief §5) routes by
  decision class (curated `DecisionClass` taxonomy) to an owner-involvement
  level (`Never<Pm<Notify<Ask`), defaulting per-class (seeds; owner configures
  later via the spectrum), with an authority-downgrade gate so a producer can
  never under-claim owner involvement. Per-class policy persistence is a future
  round (owner-configured autonomy knobs). **[UPDATED 2026-08-10]** the policy
  is now itself event-sourced: `DecisionPolicyChanged` events (owner via
  `POST /api/policy`) fold into `Projection.policy`, and the gate + PM derive
  involvement from it — delegated authority is durable history, not a hardcoded
  default, and is actually enforced.

No decisions are blocking active work. D2's *execution* (scripted → real
provider) remains open but is deliberately **deferred** behind the product
surface build-out (see §5); when it is picked up, any new decisions should be
recorded here.

---

# 7. Conventions & notes for the incoming agent

- **Rust style:** `rustfmt` + `clippy -- -D warnings` clean is required
  before committing. Keep clippy at zero.
- **Testing:** new behavior goes in `tests/` as integration tests using
  the public crate API. The store is tested via tempfile-backed SQLite;
  the PM loop via `pm::drive_pm` on in-memory stores (`#[tokio::test]`).
  **The policy gate is tested in `tests/policy_gate.rs`** — pure, fast,
  no store needed.
- **`EventType` is a curated enum** — extend the domain vocabulary
  deliberately. Keep the domain/telemetry separation.
- **Frontend:** TypeScript, strict mode. `frontend/dist/` is gitignored —
  never commit built output; `build.rs` covers the compile-time contract.
  Always rebuild the SPA (`npm run build`) before rebuilding the binary if
  the UI changed. Keep the SPA's TS types in `api.ts` in sync with
  `projection.rs` field names.
- **Single binary for now.** Do not pre-split crates
  (domain/application/infrastructure/web/cli) until a boundary proves
  itself (brief §37).
- **Commit style:** conventional (`feat:`, `chore:`, `docs:`, `fix:`),
  focused commits. The owner commits; leave the tree clean.
- **The owner is an experienced developer (20+ yrs)** and makes the big
  decisions — raise tradeoffs, don't unilaterally pick majors. The SPA
  choice (D3) is an example of the owner overriding a recommendation;
  record such reversals in §6.

---

# 8. Toolchain / environment notes

- Rust via rustup (standalone at `~/.cargo/bin`, self-updatable; snap
  rustup is shadowed and should not be used — do not re-enable it).
- Node v22 + npm 10 for the frontend (already used; `frontend/node_modules`
  is gitignored; `package-lock.json` IS committed).
- Editor: VS Code with rust-analyzer + CodeLLDB + crates + Even Better
  TOML. Open the project root `/home/ben/casting`.
- The `cast run` dev workspace lives at `/home/ben/casting-workspace/`
  (artifact repo at `proj/`, state in `state/`) — always OUTSIDE the source
  tree, per D5; the D5 self-identity guard refuses any in-tree repo (`.dev/`
  is no longer used — do not create one). The smoke workspace stays at
  `.tmp-demo/` (gitignored; `cast init`/`cast smoke` don't cross the
  ownership boundary).
- Known session quirk: if `write_file` starts failing with `ENOENT`, its
  working directory may be stuck on a deleted temp folder — fall back to
  shell-based file creation (`cat > file <<'EOF'`) or restart the session.