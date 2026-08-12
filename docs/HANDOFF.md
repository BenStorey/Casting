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
> `cast run` takes `--project <dir>` (state collocated in the gitignored
> `<dir>/.casting/`) and ensures a real git repo at
> startup; a git observer turns raw branches/commits/merges into semantic domain
events; the projection renders ChangeSets; and `/api/provenance/*` answers
"why does this code exist?". **204 tests** (6+4+12+3+14+6+11+5+2+8+5+10+4+5+12+10+6+4+4+5+5+5+3+3+7+5+8+6+3+1+4+5+3+3+3+2),
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
- `docs/HARNESS.md` — harness responsibilities (fault isolation, context,
  tracing, checkpointing, cost attribution, concurrency, escalation, sandboxing,
  backpressure): what we own vs. borrow; what's already the event-sourced core.
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
   guard (refuses to operate on the repo that built the binary), a collocated
   `.casting/` state dir (self-ignored by git; one `--project` param), path
   sandboxing (`resolve_under`), and one pinned Git runner. `cast run` now
   requires `--project <dir>`. Proven by 10 tests; prerequisite for the Git
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
     ensures a real git repo exists at `--project <dir>` (git-init if missing);
     the preflight banner shows the git init, a true HEAD, and the current
     branch.
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
│   ├── actions/                    <- PM action vocabulary + policy gate (ADDENDUM §16); mod.rs facade over
│   │   │                              action.rs (enum) / policy.rs (gate) / events.rs (to_events) / owner.rs
│   ├── types.rs                    <- shared derived DATA types (Agent, Task, Decision, …) moved out of
│   │                                  projection.rs (used across src/ + tests)
│   ├── triage.rs                   <- deterministic external-request triage (single source of truth)
│   ├── auth.rs                     <- owner bearer-token auth (opt-in, CAST_OWNER_TOKEN)
│   ├── store.rs                    <- EventStore trait (append, read_since, latest_sequence)
│   ├── sqlite_store.rs             <- SQLite impl (WAL, append-only, per-project sequences)
│   ├── cursor.rs                   <- durable per-consumer cursors
│   ├── projection.rs               <- current-state projections derived from the log (§2.1)
│   ├── plan.rs                     <- Priority + Project Plan view (derived current plan)
│   ├── directive.rs                <- Project Directives = governance layer (docs/INTENT.md)
│   ├── cast.rs                     <- role catalog + default cast = team composition config
│   ├── context.rs                  <- per-agent Context Assembler (SEMANTIC_EVENTS §21)
│   ├── mental.rs                   <- operating picture: "what the models are seeing" (/api/model)
│   ├── persona.rs                  <- persona/CV rendering (brief §2.2)
│   ├── orchestrator.rs             <- D2 seam: Orchestrator trait + MockOrchestrator (real LLM off)
│   ├── integrity.rs                <- write-time event-stream precondition enforcement
│   ├── setup.rs                    <- setup engine (SetupSpec + SetupPlan, idempotent onboarding)
│   ├── snapshot.rs                 <- projection snapshots (pure read optimization, never authoritative)
│   ├── replay.rs                   <- event-stream dump + integrity verify (`cast log`)
│   ├── reconciler.rs               <- drift reconciler: cursor-gated "every N events" cleanup
│   ├── backend.rs                  <- storage backend factory (sqlite | postgres)
│   ├── postgres_store.rs           <- PostgresBackend (EventStore+CursorStore+SnapshotStore)
│   ├── pm.rs                       <- simulated PM control loop + shared AppState (§2.2)
│   ├── web.rs                      <- axum facade: `mod routes; pub use routes::router` (§2.3)
│   ├── web/routes/                 <- handlers + DTOs by concern: auth, setup, state, inbox, intake,
│   │   │                              advisor, owner, provenance, views, static_files
│   ├── workspace.rs                <- ownership boundary: self-identity guard + git runner (D5)
│   ├── git_observer.rs             <- semantic Git events from raw repo state (Git inc 2)
│   ├── provenance.rs               <- "why does this code exist?" queries (Git inc 4)
│   └── main.rs                     <- `cast` CLI (init, smoke, run, log)
├── tests/
│   ├── event_store.rs              <- 6 integration tests (headless core)
│   ├── vertical_slice.rs           <- 3 integration tests (projection + PM loop)
│   ├── policy_gate.rs              <- 12 unit tests (PmAction validation + assignee checks + JSON)
│   ├── ownership_boundary.rs       <- 10 tests (self-identity guard, sandbox, state-dir, ensure_repo)
│   ├── git_observer.rs             <- 11 tests (semantic Git events, ChangeSet auto-derive, merge)
│   ├── provenance.rs               <- 3 tests (provenance chain: commit → task → requirement → owner)
│   └── task_review.rs              <- 5 tests (review lifecycle: InReview, approved→Done, rejected→rework)
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
`project-demo` in `main.rs`; the single-project model means the binary only
ever serves this one project):

- `GET /api/state` — the current projection (drives the UI).
- `GET /api/events?after=N` — raw event slice (activity/catch-up).
- `GET /api/events/stream` — Server-Sent Events, pushes every appended
  event live (SSE built with `futures::stream::unfold` on the broadcast).
- `POST /api/request` `{source, external_id?, title, body?, reporter?, labels?,
  url?}` — an EXTERNAL request (a GitHub issue/PR, email, web) enters the
  product's intake surface; triaged deterministically (classification + severity)
  and recorded with provenance (NOT the owner's own intent). Returns the event.
- `POST /api/diagram` `{title?, data}` — save a diagram drawn in the app; `data`
  is the serialized tldraw JSON captured directly from the editor at save time
  (no export/re-upload), stored as a durable `DiagramSaved` artifact.
- `POST /api/advisor/message` `{body}` — owner→advisor; appends to the PRIVATE
  advisor thread, isolated from PM context until a handoff.
- `POST /api/advisor/handoff` `{summary, title?}` — turn the advisor thread into
  an `AdvisorHandoff` briefing (source "advisor") the PM DOES read.
- `GET /api/inbox` — decisions awaiting the owner.
- `POST /api/message` `{body}` — owner → PM; persists a durable
  `MessageSent` and wakes the PM.
- `POST /api/brief` `{source?, subject?, title?, body, assets?}` — the owner
  imports EXTERNAL advisor content (e.g. a pasted ChatGPT plan) as an ADVISORY
  briefing. Explicitly NOT authoritative: `source` marks provenance, and it can
  inform context but never sets rules. Returns the stored event.
- `POST /api/decision` `{decision_id, subject, approved, note}` —
  persists `DecisionMade` (actor = Owner).
- `POST /api/policy` `{class, involvement}` — owner configures a decision
  class's owner-involvement; persists `DecisionPolicyChanged` (actor = Owner),
  the event-sourced autonomy config the gate enforces.
- `POST /api/directive` `{id, kind, statement, scope, strength}` — owner sets
  project governance directly; persists `ProjectDirectiveCreated` (actor =
  Owner). If a strength is omitted it defaults to `required`.
- `POST /api/hire` `{role_id}` — owner adds an agent of a curated catalog role
  to the cast; persists `AgentHired` (actor = Owner). Unknown role → 400.
- `POST /api/login` `{token}` — verify an owner token (200 ok / 401). When
  `CAST_OWNER_TOKEN` is set, the owner-mutating endpoints (`message`/`decision`/
  `policy`/`directive`/`hire`) require `Authorization: Bearer <token>`; reads
  stay open.
- `GET /api/setup/status` — `{ configured, roles[] }` for the first-run wizard
  (`configured` = a cast is hired, not just the seed PM; `roles` = the catalog).
- `POST /api/setup` `{name, objective, cast:[role ids], owner_token?}` — the
  first-run submit: hire the cast (idempotent), persist the token, then fire the
  owner's objective as a message so `plan_onboard` kicks off. Shares ONE engine
  with `cast init`.
- `GET /api/context/{actor}` — the assembled operating context for an agent
  (or "owner"/"pm"): objective, ranked priorities, their tasks, the governance
  directives that apply to them, risks, and open decisions (Context Assembler).
- `GET /api/persona/{agent_id}` — the derived persona/CV card for a hired
  agent (role, title, current/completed tasks, applicable directives); 404 if
  the agent isn't hired.
- `GET /api/model` — the **operating picture** ("what the models are seeing"):
  objective, ranked priorities, governance (active directives + decision policy
  + open decisions), knowledge (active opinions + superseded-opinion audit +
  facts + assumptions + constraints), context (open risks/requirements, task
  counts, active agents), the per-actor operating contexts each model is handed,
  and mechanical `drift_signals`. The owner's debug surface for "why is it
  prioritizing that way?" (pure derivation, no LLM).
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
**Client state (owner decision 2026-08-10): Zustand** (`frontend/src/store.ts`).
The Rust backend is the single source of truth; the store holds the `/api/state`
snapshot and treats the SSE stream as "something changed → refetch" — it does
NOT re-derive the projection in TypeScript (that would create two authorities).
`useCastStore` exposes `{state, inbox, error, streamReady, refresh, start}`;
`App.tsx` selects slices via `useCastStore((s) => …)`.
**UI look (owner direction):** Tailwind 4 + shadcn/ui (Radix-based, copy-in,
fully themeable via CSS variables) — NOT antd/MUI (enterprise/finance chrome).
CSS-first Tailwind v4 config (`@import "tailwindcss"` + `@theme` with oklch
tokens in `index.css`; `@tailwindcss/vite` plugin, no config file). Design
tokens are the `:root` CSS vars; edit them to re-theme the whole app.
**Frontend stack is current-gen (2026-08-10 upgrade):** React 19, Vite 8,
TypeScript 7, Tailwind 4.3, latest shadcn. `src/components/ui/*` are shadcn
components; App.tsx applies `Tabs` nav + Inbox unread `Badge`, `Button`/`Input`
composer, approve/reject `Button`s.
**First-run wizard:** when no cast is hired yet (only the seed PM), the SPA
shows `SetupWizard.tsx` — an **in-character, 4-step onboarding** where the PM
(Sarah Chen) introduces herself by avatar and explains the steps: meet the
team → pick the cast (role buttons show name + avatar) → set the objective →
optional owner token. It drives the SAME setup engine as `cast init` via
`/api/setup`.
**Cast identities & avatars DONE 2026-08-10:** `frontend/src/cast.ts` gives each
PM + catalog role a stable name, role title, persona, 3-line CV, and avatar.
Team view shows avatar + CV. Avatars are `/avatars/*.svg` monograms now; cartoon
PHOTOS are staged but blocked until the image backend is reachable (OpenRouter
image model 403 → enable OpenAI image access or set
`OPENROUTER_IMAGE_MODEL=google/gemini-3-pro-image`; then drop-in .png).
**In-app drawing DONE 2026-08-10 (Excalidraw):** a **Sketch** tab runs an
**Excalidraw (0.18.x, MIT)** canvas for freeform architecture diagrams / UI
sketches. Save serializes the scene DIRECTLY (`serializeAsJSON`) and POSTs to
`/api/diagram` — no download/re-upload — a durable, reloadable `DiagramSaved`
artifact in `proj.diagrams`. Lazy-loaded (React.lazy). **Chosen over tldraw for
LICENSING:** tldraw needs a paid production license ($6k/yr commercial, and
even OSS downstream users each need one); Excalidraw is MIT — free forever, no
keys/watermarks. (Also chosen over React Flow: it's node/edge-bound and wrong
for freeform UI sketches.) Known: 2 HIGH npm-audit items are transitive pins
inside Excalidraw (lodash-es, nanoid) — low real-world risk here, track not
block.
**Direction Advisor DONE 2026-08-10:** an **Advisor** tab (lazy) — Amara Okafor,
Strategic Advisor, a special SECOND role the owner talks to directly (after the
PM). Free chat is **isolated** from PM context by design (`advisor_thread`);
only `Hand off to the PM` converts it into an AdvisorBriefing (provenanced
"advisor") the PM reads. She reads high-level state + asks/advises — cheap to run
top-tier because low-volume. Her replies are D2 (LLM).
**Cockpit polish DONE 2026-08-10:** all six views on full shadcn components
(Board task-cards + status Badges, Team Card grid, Decisions/Inbox Cards +
approve/reject Buttons, Chat Card shell). **Activity** is a genuine live event
log — powered by `/api/events` (in the Zustand store), newest-first, showing
sequence + event_type + actor — not a reconstruction from the projection.
**Context-assembly scoring DONE 2026-08-10:** `AgentContext.scored_priorities`
ranks each priority's relevance to the receiving actor (own-task + urgent/
blocked items highest), surfaced in `/api/model`; owner/PM see everything.

## 2.5 `cast` CLI / setup wizard (SINGLE-project)

**Casting is SINGLE-PROJECT (owner decision 2026-08-12).** The binary relates
to exactly ONE project — the dir you pass to it. There is **no** multi-project
registry and **no** project name; the home-dir `~/.casting/projects.json`
registry was **removed**. Rationale: if projects were linked in one window, the
failure of one could break the others — that must not be possible. The cloud
service later will be the multi-project-in-one-window *differentiator*; the
local-first binary stays strictly one-project. **Multi-user is also NOT
supported** (git is the sharing surface; each owner runs their own setup).
Per-project *state* lives collocated in `<repo>/.casting/` (gitignored).

- `cast init <project-dir> [--interactive] [--name=..] [--objective=..]
  [--cast=engineer,qa] [--owner-token=..] [--directive=stmt|scope]` — the
  setup wizard/engine (owner decision: CLI + first-run UI share ONE engine, no
  second copy). Flag mode is scriptable/headless; `--interactive` prompts for
  any missing field on stdin. Creates `<project-dir>/.casting/` (self-ignored),
  writes `ProjectCreated`, hires the chosen cast roles (`AgentHired`), optional
  starting directives, persists `config.json` (name + owner token), writes a
  no-secrets `casting.example.json` template to the repo root. Idempotent. Does
  NOT fire the objective (that stays the owner's first UI message →
  `plan_onboard`).
- `cast run <project-dir> [--db <selector>] [--selfhost]` — boot the ONE
  project directly (no registry, no name resolution). Enforce the ownership
  boundary (D5), open/create stores in `<project-dir>/.casting/` (collocated +
  gitignored), seed if empty, spawn the PM loop, serve API + embedded UI at
  `http://127.0.0.1:8080` (`CAST_ADDR` env overrides). `--db <selector>` (or
  `CAST_DB` env) selects the storage backend: `sqlite` (default) or a libpq
  Postgres string (hosted). No prior `cast init` needed (bare run auto-hires
  the default cast on first message). Owner auth: token from `config.json`
  first, else `CAST_OWNER_TOKEN`.
- `cast smoke <dir>` — headless-core smoke test (append/replay/cursor).
- `cast brief <project-dir> [--subject S] [--source SRC] [--title T] <file|->` —
  import EXTERNAL advisor content (a text file, or stdin via `-`) as an
  ADVISORY briefing: it can inform context but NEVER sets rules (provenance
  `source` keeps it distinct from the owner's own intent).
- `cast request <project-dir> [--source SRC] [--reporter R] [--label L] <title>` —
  receive an EXTERNAL request (issue/PR) into the product's intake surface;
  triaged deterministically and recorded with provenance.
- `cast log --db <events.db> [--project <id>] [--verify]` — dump / verify.

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
cargo test           # 6+4+12+3+14+6+11+5+2+8+5+10+4+5+12+10+6+4+4+5+5+5+3+3+7+5+8+6+3+1+4+5+3+3+3+2 = 204 tests (all pass; ~0s)
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
state persists collocated in a **gitignored `.casting/` dir** inside the
artifact repo (the ownership boundary D5 / `docs/OWNERSHIP_BOUNDARY.md`),
so it never shows as pending changes.

> **⚠️ Do not use an in-tree workspace** (e.g. `.dev/proj`). The D5
> self-identity guard refuses any repo inside the embedded source root
> (`/home/ben/casting`), so `cast run --project .dev/proj ...` fails at boot
> unless you pass `--selfhost` (which is the wrong semantic — it records the
> run as building Casting). The artifact repo must live outside the Casting
> source tree, exactly as `/home/ben/casting-workspace/proj` is set up. See
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

> ## ⚡ OWNER DIRECTIVE (2026-08-10): build the rest of the product FIRST
>
> Sequencing is now explicit. **Before** either of these two things, build the
> rest of the deterministic product surface first:
>
> 1. **Do NOT expand to multiple projects / multiple users yet.**
> 2. **Do NOT wire any real LLM integration (D2) yet.**
>
> Instead, keep building the deterministic product surface around the seam —
> the **event / decision / state core is the product** ("this is what we live or
> die by"). Keep every increment LLM-free, deterministic, and tested. Only reach
> for multi-project / multi-user / real-LLM **once the product surface is
> complete**. The real LLM stays a *thin client over a complete product*, never
> a model racing a half-built scaffold.

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

> **2026-08-10 owner note:** this is the section to keep working. The event /
> decision / state core IS the product — keep maturing it and adding product
> surface here before touching multi-project, multi-user, or a real LLM (per the
> ⚡ directive at the top of this roadmap).

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
3. ~~**Decision lifecycle maturity / anti-thrash.**~~ **Partial — supersession
   DONE 2026-08-10.** `DecisionStatus::Superseded` + `Decision.superseded_by`;
   a decision can be superseded by a newer one (never deleted, history
   preserved) via `PmAction::SupersedeDecision`; superseded decisions drop out
   of the plan's open decisions. *Reactive* anti-thrash (the PM deciding *when*
   to supersede / not re-propose) needs the LLM/PM reasoning (deferred, D2).
4. **Event-stream integrity + tooling** — **DONE 2026-08-10.** `replay::dump`
   (raw one-line-per-event history) + `replay::verify` (checks sequence
   contiguity and "DecisionMade follows DecisionProposed" / "TaskCompleted
   follows TaskCreated"). CLI: `cast log --db <events.db> [--project <id>]
   [--verify]`. Advisory, not DB-enforced.

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

**Governance layer (Project Directives) — DONE 2026-08-10** (docs/INTENT.md).
The third pillar of the conceptual model alongside intent and state — *how we
operate* as first-class, event-sourced state, not prompt text:

- **Directive model** (`src/directive.rs`): `kind` (policy/constraint/
  principle/practice/preference/objective), `strength`
  (required>strong>recommended), `status` (active/suspended/superseded/
  expired), `scope`, and supersession (replaced by another id, history kept).
- **Lifecycle events** (`ProjectDirectiveCreated/Suspended/Resumed/Superseded/
  Expired`) reduced into `Projection.directives`.
- **Authority gate**: only owner/PM/system may change governance; a plain agent
  can only raise an Observation (propose). Mutations on a missing directive are
  rejected; supersession requires an existing active target.
- **Context resolver** `directive::relevant(projection, areas)` surfaces only
  the ACTIVE directives overlapping an agent's scope, strongest-first (the
  INTENT "exists once, surfaced per agent" payoff).
- Surfaced in the plan as `active_directives`. Governance is owner-only: the
  owner sets directives directly (`POST /api/directive`) or by approving a
  PM-proposed `GovernanceChange` decision. The PM/agents may PROPOSE a change
  (via the decision pipeline) but only the owner authorizes it.

Draws directly on the delegated-authority machinery: governance edits flow
through the same validated gate as decisions, and no agent can silently change
the rules it operates under.

**Cast & TeamChange — DONE 2026-08-10** (different CEOs build different casts):
- `src/cast.rs`: curated `ROLE_CATALOG` (engineer/qa/security/devops, each with a
  real governance scope — role is the atom, no separate specialization axis) +
  `DEFAULT_CAST` seed. `plan_onboard` hires the cast by role; `context::scopes_for`
  derives an agent's governance scope from its catalog role (accurate per-agent
  governance, replacing string-matching).
- The "can the PM add a consultant" rule is the owner's one-line policy on the
  AddConsultant decision class: Pm = PM auto-hires, Ask = surfaces for owner
  approval. `POST /api/hire {role_id}` = owner adds a role directly;
  `ProposeConsultant` = PM proposes via the decision pipeline, applied on approval.

**D2 seam + integrity hardening — DONE 2026-08-10:**
- `src/orchestrator.rs`: the D2 contract. `Orchestrator` (assembled context +
  cause → `PmAction`s, still gate-checked) + `MockOrchestrator` prove the seam
  end-to-end with **zero live LLM / zero spend**. `AppState.orchestrator` is OFF
  by default (the real provider stays unplugged while away); `with_orchestrator`
  enables it. When enabled, `pm::respond` routes owner messages through it.
- `src/integrity.rs`: write-time event-stream enforcement. `check_append` rejects
  a DecisionMade/DecisionSuperseded without a prior DecisionProposed, and task
  lifecycle events without a prior TaskCreated, at the moment of write. Opt-in
  via `AppState::with_integrity`; the production `cast run` enables it. `cast
  log --verify` remains the full-stream advisory check.

**Next (D2, deferred until the product surface is complete — per the ⚡ directive
above):** plug the real OpenRouter provider into the `Orchestrator` seam and flip
it on in production. Keep building the deterministic surface first.

**Owner auth — DONE 2026-08-10** (scoped to auth alone; multi-project later):
- `src/auth.rs`: constant-time bearer-token guard for the owner-mutating API
  endpoints (`message`/`decision`/`policy`/`directive`/`hire`), opt-in via
  `AppState::with_owner_auth` / the `CAST_OWNER_TOKEN` env var. `POST /api/login`
  verifies the token. Reads stay open (the whole site is already behind Caddy
  basic auth in production; this is the write-authority boundary inside the app).

**Setup engine + CLI wizard — DONE 2026-08-10** (owner decision: CLI + the
future first-run UI share ONE engine, never two):
- `src/setup.rs`: `SetupSpec` (name, roles, owner token, optional starting
  directives) → `SetupPlan::build` (resolves default cast, validates roles)
  → `apply(<project>/.casting/)` idempotently writes `ProjectCreated` + hire
  cast + optional directives, and persists `config.json` (name + owner token)
  in the self-ignored `.casting/` dir. `cast init` also writes a no-secrets
  `casting.example.json` template to the repo root.
- `cast init` drives it (flag- or interactive-); `cast run` reads the persisted
  token first. `plan_onboard` no longer tops-up a setup-chosen custom cast.
- **Web first-run wizard DONE**: `/api/setup/status` + `/api/setup` + the SPA's
  `SetupWizard.tsx` drive the same engine — name, objective, role picker, owner
  token. Phone-testable.

**Postgres storage backend — DONE 2026-08-10** (owner principle: every store
read/write goes through the abstraction; Postgres is a freely-swappable
backend, we do NOT carry two concrete paths in AppState):
- `CursorStore`/`SnapshotStore` are now traits (like `EventStore`); `AppState`
  holds `Arc<dyn ...>`. SQLite = `SqliteCursorStore`/`SqliteSnapshotStore`;
  Postgres = one `PostgresBackend` implementing all three.
- `src/postgres_store.rs`: drives tokio-postgres on a dedicated background
  thread (its own runtime + connection), so the sync traits work from any
  thread incl. our tokio server (the sync `postgres` crate's nested-runtime
  limitation made this necessary).
- Runtime selection: `cast run <dir> --db <selector>` or `CAST_DB`.
  Verified against real Postgres (docker).
- `deploy/docker-compose.postgres.yml`; integration tests
  `tests/postgres_backend.rs` (real PG round-trip, full company boot+onboard).

**Knowledge layer — OPINION + FACT — DONE 2026-08-10** (owner concept: "save
down interesting facts so we don't re-derive them", in Casting-native form):
- Distinction the owner drew: knowledge worth preserving is **OPINION**
  (subjective rationale, lost unless recorded) vs objective **FACTS** (usually
  derived from state; recorded only as a point-in-time snapshot when it
  matters). We build both; the LLM/gate decides which.
- `OpinionRecorded` {category, statement, recorded_by, supersedes} + `FactRecorded`
  {kind, statement, recorded_by, recorded_at}; projected to `proj.opinions` /
  `proj.facts`. When the owner asked how the "currently valid" set is derived,
  supersession was made explicit (mirrors directives): `Opinion.status`
  (Active|Superseded) + `OpinionSuperseded` event + `PmAction::SupersedeOpinion`;
  readers use `proj.active_opinions()` (status==Active) for the current view,
  with the full audit trail preserved in `proj.opinions`. `RecordOpinion` /
  `RecordFact` / `SupersedeOpinion` all pass the gate.
- When D2 lands, the LLM writes learned knowledge through the same gate — the
  deterministic home for lessons/rationale/preferences.
- `docs/STORAGE_CANDIDATES.md` holds the immutable reasoning about Postgres /
  FoundationDB / etc. choices.

**Drift reconciler — DONE 2026-08-10** (owner framing: knowledge drifts rather
than going stale in a burst; keep writes simple, reconcile periodically. This is
the reusable "every N events" primitive, later also for priority/plan
re-ranking):
- `Opinion.subject` = the matching key (what an opinion is ABOUT; empty =
  ungroupable), from the Model-2 mechanics design fork.
- `src/reconciler.rs`: a reconciler CONSUMER (own cursor); `should_run` fires
  when `latest - reconciler_cursor >= interval`; `drift(projection)` finds
  same-subject Active duplicates (keeps latest, flags older); `reconcile` emits
  `SupersedeOpinion` through the gate + advances its cursor; `run_if_due` in
  the PM loop. `reconcile_interval` on AppState (default 25).
- The D2 seam later supplies the *smart* "what truly conflicts" judgment; the
  skeleton does the mechanically-obvious cleanup deterministically and
  idempotently.

**Operating picture — DONE 2026-08-10** (owner need: a single surface to dump
what the PM/agents currently believe and prioritize — for debugging a
wrong-priority PM AND for users):
- `GET /api/model` (`src/mental.rs`, `Projection::operating_model()`): the
  curated read-model — objective + ranked priorities, governance (active
  directives + decision policy + open decisions), knowledge (active opinions by
  subject + superseded-opinion audit + facts + assumptions + constraints),
  context (open risks/requirements, task counts, active agents), the per-actor
  operating contexts each model is handed (`context_for`), and mechanical
  `drift_signals`.
- Pure derivation, no LLM. The `drift_signals` field reuses the same-subject
  Active-contradiction detection as the reconciler, surfacing it to the owner
  BEFORE the reconciler's next pass.

**Cost attribution — DONE 2026-08-10** (HARNESS #6; the one harness
responsibility worth designing NOW, per the ⚡ directive that the LLM stays a
thin client over a complete product — so when D2 wires real providers, spend is
attributable from day one):
- `Orchestrator::plan` returns `PlanOutput { actions, metering }`; a provider
  call reports `CostMetering` (agent_id, task_id, model_tier, tokens, USD).
- The PM lands it as a `CostIncurred` event → `proj.spend` (CostEntry),
  aggregated via `total_spend_usd` / `spend_by_agent`.
- `/api/model` surfaces `spend` (total + per-agent) so the PM's budget concern
  has real data. MockOrchestrator meters (~$0.0018) on planning calls so the
  seam is tested end-to-end with the LLM still off.

**External advisor briefings — DONE 2026-08-10** (owner: "a way to dump stuff
in that's generated outside Casting ... that doesn't set the rules." Fixes the
provenance failure where an imported .md became authoritative by default):
- `AdvisoryBriefingImported` event + `proj.briefings` (Briefing{source, subject,
  title, body, assets, brought_in_by, status Active|Superseded, supersedes}).
  `source` marks provenance so advice is never confusable with the owner's own
  intent; supersession lets stale advice decay instead of dominating.
- `PmAction::ImportBriefing`; `cast brief <project> [--subject S] [--source SRC]
  [--title T] <file|->`; `POST /api/brief`.
- `/api/model` surfaces them under `knowledge.briefings` (AdvisoryView: active +
  superseded + count), clearly separate from governance and Casting's own beliefs.
- The rule: advisory can INFORM context, never SETS rules (directives remain the
  only authority mechanism). Images/diagrams are reference assets (caption +
  path/URL); vision-derivation is a D2 item.

**ExternalRequest intake — DONE 2026-08-10** (owner: "eventually all feature and
bug requests go to the PM"): the deterministic half of the autonomous-company
vision. A user's GitHub issue/PR (or email/web) becomes the product's third
intake surface, alongside Requirement (owner) and AdvisoryBriefing (advisor):
- `ExternalRequestReceived` event + `proj.external_requests` (provenance:
  source, external_id, reporter, labels, url) with deterministic triage
  (`triage_request`: classify bug/feature/security + severity + dedup by
  external_id or lowercased title).
- `PmAction::ReceiveExternalRequest`; `POST /api/request`; `cast request`.
- `/api/model` surfaces the intake inbox (`requests.open_count` + triaged list).
- The LLM judgment ("is this a real bug? how bad?"), fixing, and releasing are
  D2 + policy-gated (DecisionPolicy can allow auto-fix/release per class).

**Human-as-consultant — DONE 2026-08-10** (owner: "let the HUMAN implement a
feature, working through their own harness; a task assignable to the owner is
all we need"): a task can now be assigned to the owner (`OWNER` pseudo-assignee,
`is_valid_assignee`), so the owner may take it on personally and deliver via
git — which the observer already records (ChangeSets + provenance). This is the
delivery-mirror of ExternalRequest (work going OUT and back through git, not in
for triage). Explicitly NOT a full agent / NOT multi-user; just "the owner can
be the doer." Delivery machinery was already in place; the change was the
intent/gate.

**Direction Advisor — DONE 2026-08-10** (owner: "a permanent advisor who runs the
highest model but is cheap because it only reads high-level state and asks;
chat freely without affecting PM context, with a mechanism to hand off to the
PM"): a special second owner-interaction role, separate from the PM. Its chat is
ISOLATED (`advisor_thread`) — advising never pollutes PM context — until the
owner explicitly hands off (`AdvisorHandoff` → an AdvisoryBriefing provenanced
"advisor" the PM reads). Rationale for the separate role: (1) altitude — the PM
is inside task machinery; the advisor is elevated above it; (2) model-tier
economics — low-volume, top-tier is cheap precisely because it's NOT fused with
the PM's constant loop. Her replies are D2 (LLM); the seam + isolation are built.

**In-app drawing — DONE 2026-08-10** (owner: "a canvas to sketch architecture
diagrams or UI quickly, even if just boxes"): a **Sketch** tab runs an
**Excalidraw 0.18.x** freeform canvas. Save serializes the scene DIRECTLY
(`serializeAsJSON` → `POST /api/diagram`, durable `DiagramSaved` artifact in
`proj.diagrams`) — no export/upload. **Excalidraw (MIT) was chosen over tldraw
for licensing**: tldraw demands a paid production license ($6k/yr; each OSS
downstream user needs one), which is incompatible with an open-source product.
Lazy-loaded so the heavy editor chunk only downloads when the tab opens.
Diagrams are durable visual artifacts (like briefing assets), reloadable,
surfaceable; vision-derivation of them is a D2 item.

Then, only after the core matures:
5. ~~**Task `review` status**~~ — **DONE 2026-08-10**: `TaskStatus::InReview` +
   `Task.review` (verdict for provenance); `TaskReadyForReview` / `TaskReviewed`
   events; `RequestReview` / `ReviewTask` through the gate (assignee submits,
   reviewer must be hired, only InReview task can be ruled on; rejected -> back
   to Working for rework). Onboarding runs core work through a real QA review;
   persona highlights only *verified* (approved) completed work.
6. ~~**Persona / CV rendering** (brief §2.2)~~ — **DONE 2026-08-10**
   (`Projection::persona_for`, `GET /api/persona/{id}`): the friendly identity
   layer, a pure renderer over the underlying agent configuration.
7. ~~**Auth + multi-project** (brief §2.1/§31).~~ **RETHOUGHT 2026-08-10, then
   MULTI-PROJECT REMOVED 2026-08-12**:
   - **Multi-user is DROPPED.** Git is the sharing surface — each human runs
     their own Casting setup. Single-owner auth (token today; password/signed
     key later) is always enough. No users/roles/permissions to build.
   - **Multi-project is REMOVED (owner decision 2026-08-12).** Casting is
     strictly SINGLE-PROJECT: the binary relates to exactly ONE project (the
     dir you pass to `cast run <dir>`). The home-dir registry
     (`~/.casting/projects.json`, `cast add/remove/list`, `cast run <name>`)
     was **deleted**. Rationale: linking projects in one window means the
     failure of one can break the others — that must not be possible. The
     cloud service later will be the multi-project-in-one-window
     *differentiator*; the local-first binary stays one-project. Owner auth
     is done.
8. ~~**Cost capture** (brief §6) — spend/budget/forecast.~~ **DEPRIORITIZED**
   (not urgent).

Design note: every one of these is LLM-free and independently testable, exactly
like the Git slice. Keep `EventType` a curated enum and extend it deliberately.

> **2026-08-12: FULL CODE REVIEW + REFACTOR (10k+ LOC)** — a parallel 3-agent
> review + restructuring pass. Correctness fixes: the policy gate is now
> FAIL-CLOSED (no `_ => Ok(())` catch-all; every create-action enforces id
> uniqueness via `DuplicateEntity`); external-request triage is single-source in
> `src/triage.rs` (was duplicated, had drifted); `DecisionMade` shape unified;
> all projection access routed through snapshot-aware `AppState::projection()`;
> stale `tldraw`→`Excalidraw` docs fixed. Structure: `src/actions.rs` split into
> `src/actions/` (mod.rs facade + action/policy/events/owner); 28 data types
> moved from `projection.rs` to `src/types.rs`; `src/web.rs` split into
> `src/web/routes/` (auth/setup/state/inbox/intake/advisor/owner/provenance/
> views/static_files) + shared `append_json`; `main.rs` bootstrap deduped into
> `open_state()/setup_state()`. **204 tests**, clippy 0, fmt clean. Deferred:
> `pm.rs` `plan_onboard` policy extraction, `ev`/`linked`/`str` renames, a typed
> `TriageVerdict` return.

### Local Git (addendum §28, 18–27) — COMPLETE (4 increments built)
All four increments are built, tested, and committed. The Git slice is
fully deterministic (no LLM anywhere). Everything uses the pinned git runner
from `src/workspace.rs` and `resolve_under`.
**Ownership boundary prerequisite (D5 / `docs/OWNERSHIP_BOUNDARY.md`) is LANDED**
— `src/workspace.rs` provides the self-identity guard, path sandboxing, the
single pinned git runner (`Workspace::git_command()`), and the collocated
`.casting/` state dir (gitignored), so the Git workflow can never accidentally
target the Casting source. Everything below uses that runner and `resolve_under`.
No LLM anywhere; this is fully deterministic.

Proceed one increment at a time, in this order:

1. **Boot-time repo management.** `cast run --project` ensures a real git repo
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
- ~~**Task `review` status**~~ **DONE** (2026-08-10).
- **Multi-user + LLM integration** — **explicitly deferred until
  the rest of the product surface is complete** (per the ⚡ directive at the top
  of this roadmap). Owner auth alone is done. **Multi-project is not deferred —
  it is REMOVED** (owner decision 2026-08-12): Casting is strictly
  single-project; the cloud service later will be the multi-project
  differentiator.

### Later: Postgres, realtime dashboard polish, external owner messaging
(Telegram/WhatsApp), context-assembly scoring — these remain LLM-free surface
and are fair game to build once the core candidates above are exhausted. If in
doubt, work the core candidates first.

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