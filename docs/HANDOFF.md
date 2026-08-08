# Casting — Project Handoff

Status: Handoff from current session to next agent/session
Date: 2026-08-08
Author: Hermes (acting on behalf of owner, Ben)

This document tells an incoming agent everything it needs to know to pick
up Casting: what the product is, what exists today, how to build/test/run
it, what has been decided, and what to do next. **Read the docs listed
here — they are the authoritative design.** This file is the map, not the
design itself.

---

# 1. Quick orientation

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
- `docs/INITIAL_PITCH.md` — the original rough idea (context/history).
- `docs/ENGINEERING_NOTES.md` — scoping notes + open decisions (D1–D4).

---

# 2. What exists today

The repository is a **headless, LLM-free foundation** — slice one of the
brief's first vertical slice. It proves the core architecture without any
LLM, server, persistence-of-projections, or UI. It is fully working,
tested, and committed.

## Repository layout

```
/home/ben/casting/                  <- PROJECT ROOT (this is the git repo root)
├── Cargo.toml                      <- single binary; lib crate "casting" + bin "cast"
├── src/
│   ├── lib.rs                      <- module root
│   ├── event.rs                    <- typed domain events (Actor, EventType, Aggregate, Metadata)
│   ├── store.rs                    <- EventStore trait (append, read_since, latest_sequence)
│   ├── sqlite_store.rs             <- SQLite impl (WAL, append-only, per-project sequences)
│   ├── cursor.rs                   <- durable per-consumer cursors
│   └── main.rs                     <- `cast` CLI (init, smoke)
├── tests/event_store.rs            <- 6 integration tests (all passing)
├── docs/                           <- all design docs (see §1)
└── .vscode/tasks.json              <- build/test/lint/smoke tasks
```

## What each module does

- **`event.rs`** — The typed domain-event model from the brief §11:
  `Event { event_id, project_id, sequence, timestamp, actor,
  event_type, aggregate, data, metadata{correlation_id, causation_id,
  agent_run_id} }`. Only DOMAIN events; telemetry is deliberately excluded
  (brief §12). The `EventType` set is small and curated.
- **`store.rs`** — The `EventStore` trait: `append`, `read_since(project,
  after_seq)`, `latest_sequence`. Database-independent (brief §10) so
  Postgres can be added behind the same trait later.
- **`sqlite_store.rs`** — SQLite backend. WAL mode. Append-only `events`
  table with `UNIQUE (project_id, sequence)`. Sequence assigned as
  `MAX(sequence)+1` per project (serialized via a Mutex for slice one).
- **`cursor.rs`** — `CursorStore` with durable `(project_id, consumer,
  last_seen)` positions. Every consumer (PM, agents, projections) resumes
  from its cursor (brief §16–17, addendum §2).
- **`main.rs`** — `cast init <dir>` (creates `.casting/` with `events.db`
  + `cursors.db`) and `cast smoke <dir>` (appends sample domain events,
  replays them, exercises cursor advance + durable resume).

Both stores open separate DB files (`events.db`, `cursors.db`) under
`.casting/`. (A future step could unify them into one file — see roadmap.)

---

# 3. Build, test, run

Environment: Rust **1.97.1** (stable), cached via rustup. No MSRV concern.

```
cd /home/ben/casting
cargo build          # builds target/debug/cast
cargo test           # 6 integration tests (all pass)
cargo clippy --all-targets -- -D warnings
cargo fmt            # format (rustfmt)
```

Run the CLI:
```
cargo build
./target/debug/cast init .tmp-demo/demo-project
./target/debug/cast smoke .tmp-demo/demo-project   # run 1: cursor 0 -> 6
./target/debug/cast smoke .tmp-demo/demo-project   # run 2: cursor resumes at 6 -> 12
```
The second smoke run starting its cursor at seq 6 (not 0) is the
durable-resume behavior — run it twice to see it.

In VS Code (opened at `/home/ben/casting`), use Terminal > Run Task, or
Ctrl+Shift+B (Build). Tasks: Build, Build (release), Test, Test (single,
prompts for a filter), Lint (clippy), Format, Run cast smoke test.
Required extensions: rust-analyzer, CodeLLDB, crates, Even Better TOML.

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
   truth (brief §16–17).
5. **The PM is a control loop, not a chatbot**, with a durable cursor and a
   persistent plan (addendum §1–2, 8).
6. **Wake ≠ act.** Never invoke the LLM per event. Coalesce via Tier-0/1
   interrupts + Tier-2 drain (PM_INVOCATION_TRIGGERS.md). This is a hard
   cost rule for day 1.
7. **Git owns artifact truth; Casting owns organizational truth; the
   integration owns provenance** (addendum §30). "Git knows what code
   exists. Casting knows why it exists."
8. **Persona/CV layer is a pure presentation of the underlying agent
   CONFIGURATION** (model, context, tools, capabilities, permissions,
   budget). Ship the technical model first; bolt the personality on later.
9. **Anti-goals for now:** no Kafka, K8s, Temporal, EventStoreDB, message
   brokers, Telegram/WhatsApp, agent marketplace, Jira compat, GitHub
   integration first. No huge dashboard. Prove the organizational loop first
   (brief §43).

---

# 5. Roadmap / what's next

The roadmap follows the brief's engineering priorities (§45) and the
first-milestone checklist (§46). The immediate next steps, in order:

### Next: the first vertical slice (simulated company, per brief §36)
Still heads the right direction — the next increment should NOT introduce a
real coding swarm. Build a tiny SIMULATED software company:

1. **A simulated PM** that turns owner input ("Build me a todo app") into
   real domain events: `RequirementCreated` → `TaskCreated` →
   `TaskAssigned` → (`TaskStarted` → `TaskCompleted`) → `ObservationCreated`
   → `DecisionProposed` → `OwnerDecisionRecorded`.
2. **Agent projection + task projection** — derive current state (kanban-
   style board, team list) from the event log. Start recomputable; do NOT
   store drifting projections yet.
3. **Owner ↔ PM messaging** — an inbox where the owner sees what needs a
   decision and replies; the reply becomes a durable `OwnerDecisionRecorded`
   event (brief §21).
4. **A minimal web UI** (see D3 below) rendered from projections: chat,
   tasks/kanban, activity stream, decisions.
5. Wire the PM's wake logic per `PM_INVOCATION_TRIGGERS.md` (even a dumb
   version: wake/coalesce, process everything since cursor, emit structured
   actions, sleep).

**Success test (from brief §46):** run the thing, meet the PM, tell it what
you want, see requirements/tasks appear, see agents appear, see tasks move,
make a decision, see it recorded permanently, reload — everything still
present and the current state explainable from history.

### Then: real LLM wiring (D2 below)
Only after the simulated loop feels right. Introduce a real provider behind
a thin client, still driving the same event loop. The PM becomes an actual
LLM control loop producing structured proposed actions validated by a policy
layer before execution (addendum §16).

### Then: real Git (addendum §28, 18–27)
Local Git first: `cast run` discovers/manages the repo; agents work on
isolated `casting/task-N-*` branches; Casting observes semantic Git events
(`BranchCreated`, `CommitObserved`, `MergeConflictDetected`, …) and links
provenance (commit → changeSet → task → decision → requirement → owner
intent).

### Later: Postgres, cost reasoning, decision policy engine, realtime
dashboard, external owner messaging (Telegram/WhatsApp), context-assembly
scoring, agent identity/persona rendering.

---

# 6. Open decisions (from ENGINEERING_NOTES.md)

These are intentionally unresolved; the owner is deciding. Do not silently
pick one without noting it.

- **D1 — Rust toolchain floor: RESOLVED.** 1.97.1 on this box. Done.
- **D2 — LLM boundary.** Does slice one ship with a scripted/simulated PM,
  or wire one real provider behind an env var from day one? Recommended:
  scripted loop for the harness first, thin `Anthropic`/`OpenAI` client stub
  ready. (Section 5's "next" assumes simulated first.)
- **D3 — Frontend approach.** Server-rendered HTML + tiny JS (keeps the
  "single binary / no build tools" promise) vs a real SPA build step
  (costs it). Recommended: server-rendered first.
- **D4 — Git/artifact model.** Decision (chosen in the addendum): Casting
  drives the workflow, Git owns artifacts, local Git first, GitHub later.
  Treatment is settled in ADDENDUM.md; just don't build GitHub before local.

---

# 7. Conventions & notes for the incoming agent

- **Rust style:** `rustfmt` + `clippy -- -D warnings` clean is required
  before committing. Keep clippy at zero.
- **Testing:** new behavior goes in `tests/` as integration tests using the
  public crate API. The store is tested via tempfile-backed real SQLite.
- **`EventType` is a curated enum** — when adding events, prefer extending
  the domain vocabulary deliberately over adding one-off variants. Keep the
  domain/telemetry separation.
- **Single binary for now.** The brief allows splitting crates only when a
  boundary proves itself (brief §37). Do not pre-split
  domain/application/infrastructure.
- **Commit style:** conventional (`feat:`, `chore:`, `docs:`, `fix:`),
  focused commits.
- **The owner is an experienced developer (20+ yrs)** and makes the big
  decisions — raise tradeoffs, don't unilaterally pick majors.

---

# 8. Toolchain / environment notes

- Rust via rustup (standalone at `~/.cargo/bin`, self-updatable; snap rustup
  is shadowed and should not be used — do not re-enable it).
- Editor: VS Code with rust-analyzer + CodeLLDB + crates + Even Better TOML.
  Open the project root `/home/ben/casting`.
- A known session quirk: if `write_file` starts failing with `ENOENT`, its
  working directory may be stuck on a deleted temp folder — fall back to
  shell-based file creation (`cat > file <<'EOF'`) or restart the session.
