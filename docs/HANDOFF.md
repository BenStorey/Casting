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

> **Status update (2026-08-09):** two slices are built plus the policy-gate
> seam (`src/actions.rs`), and now the **ownership boundary** (`src/workspace.rs`,
> D5 / `docs/OWNERSHIP_BOUNDARY.md`) — Casting refuses to operate on the repo
> that built it, keeps its state-dir always separate from the artifact repo, and
> pins all Git through one runner. `cast run` now takes `--repo` + `--state-dir`.
> 24 tests (6 + 8 + 3 + 7), clippy clean, slice suites run in ~0s. Read on.

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
│   ├── pm.rs                       <- simulated PM control loop + shared AppState (§2.2)
│   ├── web.rs                      <- axum server: JSON API, SSE, embedded SPA (§2.3)
│   ├── workspace.rs                <- ownership boundary: self-identity guard + git runner (D5)
│   └── main.rs                     <- `cast` CLI (init, smoke, run)
├── tests/
│   ├── event_store.rs              <- 6 integration tests (headless core)
│   ├── vertical_slice.rs           <- 3 integration tests (projection + PM loop)
│   ├── policy_gate.rs              <- 8 unit tests (PmAction validation + JSON round-trip)
│   └── ownership_boundary.rs       <- 7 tests (self-identity guard, sandbox, state-dir)
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
  `DecisionProposed` ("Database choice") and asks the owner via a message.
  Subsequent owner messages get an acknowledged reply; an
  `OwnerDecisionRecorded` gets acknowledged and (if approved) drives a
  follow-up task. Each event carries `correlation_id`/`causation_id`/an
  agent-run id, so the "why?" chain is already recorded.
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
  persists `OwnerDecisionRecorded`.
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

```
cd /home/ben/casting
cargo build          # builds target/debug/cast
cargo test           # 6 + 8 + 3 + 7 = 24 tests (all pass; slice suites run in ~0s)
cargo clippy --all-targets -- -D warnings   # keep at zero
cargo fmt            # format (rustfmt)
```

**Important:** `cargo build` embeds `frontend/dist/` at compile time
(rust-embed). That folder is gitignored; `build.rs` auto-writes a
placeholder `index.html` if it's missing, so a fresh checkout always
compiles and `cast run` always serves *something*. To embed the REAL SPA:

```
cd frontend && npm install   # once
cd frontend && npm run build # produces dist/ (tsc + vite build)
cargo build                  # re-embeds it
```

Run the whole product (single binary — this is the milestone UX):

```
./target/debug/cast run --repo .dev/proj --state-dir .dev/state  # -> http://127.0.0.1:8080
```
Open the URL, chat with the PM ("Build me a todo app"), watch the team
form and tasks move, decide on the database in the Inbox, reload — all
state persists in `.dev/state/` (kept separate from the artifact repo,
per the ownership boundary D5 / `docs/OWNERSHIP_BOUNDARY.md`).

Frontend dev (hot reload, no rebuild needed — pair two terminals):

```
./target/debug/cast run --repo .dev/proj --state-dir .dev/state  # terminal 1: API on :8080
cd frontend && npm run dev            # terminal 2: Vite on :5173,
                                      #   proxies /api -> :8080
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

### Next: real LLM wiring (D2 — the PM becomes an actual LLM control loop)
**The seam is DONE and proven:** the scripted policy in `pm.rs` already emits
the typed `PmAction`s through the policy gate in `actions.rs` exactly as a
provider would. Wiring up is now a thin OpenRouter client (day-1 provider,
Ben has a key) that returns structured `PmAction` JSON, converted into
`PlannedAction`s and fed to `run_planned`. The `plan_*` functions in `pm.rs`
are the only code to replace; the gate, cursor, drain shape, and provenance
all stay. Validate + reject belongs to the gate already — the model cannot
mutate state or burn tokens on an invalid action. Keep a `--no-llm`/scripted
fallback for offline use and `cast smoke`. (Deferred until the gate + tests
were in, per this handoff's amendment in §6.)

### Then: local Git (addendum §28, 18–27)
**The ownership boundary prerequisite (D5 / `docs/OWNERSHIP_BOUNDARY.md`) is
landed** — `src/workspace.rs` gives the self-identity guard, path sandboxing,
the single pinned git runner, and the mandatory separate state-dir, so the Git
workflow can never accidentally target the Casting source. Now:
`cast run` discovers/manages a real repo; agents work on isolated
`casting/task-N-*` branches; Casting observes *semantic* Git events
(`BranchCreated`, `CommitObserved`, `MergeConflictDetected`, …) and links
provenance (commit → changeSet → task → decision → requirement → owner
intent). Add `ChangeSet` as a first-class concept. Git drives the
workflow; Git owns artifacts; Casting owns the organization.

### Meanwhile / opportunistic
- **Auth + multi-project**: `cast run` is currently a shared
  `project-demo` with no login and no project selection. The brief's
  first-run UX (§2.1/§31) wants owner credentials + a project picker.
- **Realtime gap**: SSE only pushes *new* events; on reconnect, missed
  events aren't replayed (the UI refetches `/api/state`, so this is
  benign for the demo, but a catch-up cursor in the stream would be
  proper).
- **API 404 wart**: unknown `/api/*` paths fall through to the SPA
  fallback (return `index.html`, 200) instead of a JSON 404.
- **Task status model**: no `review` column yet (`TaskStatus` lacks it);
  add `ReviewRequested`/`ReviewCompleted` events + column when reviews
  arrive.
- **Cost capture** (brief §6): token/model/agent are already in event
  metadata shape (`agent_run_id`); capture spend early, reason late.

### Later: Postgres, decision policy engine, realtime dashboard polish,
external owner messaging (Telegram/WhatsApp), context-assembly scoring,
agent identity/persona rendering.

---

# 6. Decision log (D1–D5)

- **D1 — Rust toolchain floor: RESOLVED.** 1.97.1 on this box.
- **D2 — LLM boundary: SCRIPTED FOR NOW (by decision); seam built.** The
  vertical slice ships a deterministic scripted PM (no provider calls). The
  `PmAction` vocabulary + policy gate in `actions.rs` (built 2026-08-09)
  are the addendum §16 seam, proven by tests. **Day-1 provider: OpenRouter**
  (Ben's call; he has a key). LLM wiring is deliberately deferred until the
  gate/tests were in so any model mistake is rejected before it burns tokens
  or corrupts state.
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

Only D2's *execution* is genuinely open (scripted → real provider). Any
further open decisions should be recorded here.

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
- The `cast run` dev workspace lives at `.dev/proj` (artifact repo, gitignored)
  with its state in `.dev/state` (gitignored; kept separate per D5); the smoke
  workspace at `.tmp-demo/` (gitignored).
- Known session quirk: if `write_file` starts failing with `ENOENT`, its
  working directory may be stuck on a deleted temp folder — fall back to
  shell-based file creation (`cat > file <<'EOF'`) or restart the session.