# Casting — System Architecture

> **Audience:** Senior architect / domain expert reviewing the codebase without reading source directly.
> Every claim below has been reviewed against the codebase. Where the document describes
> design intent that differs from the *current* source, the source is authoritative.

---

## Table of Contents
1. [Philosophy & Design Principles](#1-philosophy--design-principles)
2. [Module Map & Responsibilities](#2-module-map--responsibilities)
3. [Core Data Model](#3-core-data-model)
4. [Event Sourcing Architecture](#4-event-sourcing-architecture)
5. [The PM Control Loop](#5-the-pm-control-loop)
6. [Actor Turns & Consultant Orchestration](#6-actor-turns--consultant-orchestration)
7. [Governance Layer (Directives)](#7-governance-layer-directives)
8. [Decision Policy Engine](#8-decision-policy-engine)
9. [Execution & Side Effects](#9-execution--side-effects)
10. [LLM Integration (D2 Seam)](#10-llm-integration-d2-seam)
11. [Web API Surface](#11-web-api-surface)
12. [Git Integration & Observability](#12-git-integration--observability)
13. [Workspace & Project Boundary](#13-workspace--project-boundary)
14. [Consultant Registry & Roles](#14-consultant-registry--roles)
15. [Reconciler & Drift Management](#15-reconciler--drift-management)
16. [Harness Guards (Budget, Pause, Secrets)](#16-harness-guards-budget-pause-secrets)
17. [Telegram Channel (Owner Outreach)](#17-telegram-channel-owner-outreach)
18. [Event Flow Diagram (Text)](#18-event-flow-diagram-text)
19. [Key Architecturally-Significant Design Decisions](#19-key-architecturally-significant-design-decisions)
20. [Known Gaps & Improvement Areas](#20-known-gaps--improvement-areas)

---

## 1. Philosophy & Design Principles

Casting is an **event-sourced, agent-orchestration platform** — an autonomous software company in a box. A Project Manager (PM) agent coordinates specialist consultant agents to build software. The entire system is:

- **Event-sourced:** the append-only event log is the sole source of truth. All current state (projections) is derived by folding the log and is never authoritative.
- **LLM-seamed:** the PM's "reasoning" is isolated behind the `Orchestrator` trait. The default system is *inert* — without an orchestrator, owner messages and decisions are recorded but no action is taken. LLM integration is the D2 (D-day 2) seam, not the default.
- **Policy-gated:** every proposed action (`PmAction`) flows through a pure validation gate before it becomes domain events. The gate enforces invariants (task lifecycle, authority, assignee constraints, budget limits).
- **Project-scoped:** the binary manages exactly ONE project at a time. Multi-project is explicitly deferred to a future cloud service.
- **Fail-closed:** defaults are the restrictive option (e.g., `MergeAuthority::PmMerge` requires review, `DecisionPolicy` defaults to `Ask` the owner).
- **Deterministic where possible:** triage, drift reconciliation, port allocation, worktree provisioning elaborator — all pure functions over projection state.

---

## 2. Module Map & Responsibilities

```
casting/                          (lib.rs — top-level crate)
├── src/
│   ├── main.rs                   CLI binary (cast init, cast run, cast smoke, ...)
│   ├── lib.rs                    Module declarations
│   ├── types/                    Projection entity types (Agent, Task, Decision, ...)
│   ├── event/                    Domain event model (Event, EventType, Actor, Aggregate)
│   │   ├── integrity.rs          Write-time precondition checks (opt-in)
│   │   └── replay.rs             Dump + verify event stream invariants
│   ├── store/                    Persistence abstractions + backends
│   │   ├── event_store.rs        EventStore trait
│   │   ├── cursor.rs             CursorStore trait + SqliteCursorStore
│   │   ├── snapshot.rs           SnapshotStore trait + SqliteSnapshotStore + build_from
│   │   ├── backend.rs            Backend enum (Sqlite | Postgres), from_selector
│   │   ├── sqlite_store.rs       SqliteEventStore
│   │   └── postgres_store.rs     PostgresBackend (full trait impl)
│   ├── projection/               Current-state projection (Projection + apply)
│   │   ├── graph.rs              Task lifecycle graph (Transition TABLE, GraphView)
│   │   └── port.rs               Worktree port allocation
│   ├── actions/                  PM action vocabulary + policy gate
│   │   ├── action.rs             PmAction enum, action_vocab_for, is_valid_assignee
│   │   ├── events.rs             PmAction::to_events (action→domain events)
│   │   ├── policy.rs             validate() — the pure gate
│   │   └── owner.rs              Owner-authored event shape builders
│   ├── pm/                       PM control loop & planning
│   │   ├── control.rs            AppState, run_pm, drive_pm, drain, respond, run_planned
│   │   ├── planning.rs           worktree elaborator, actors_with_work, audit event builders
│   │   ├── plan.rs               Priority, PlannedItem, ProjectPlan
│   │   ├── policy.rs             DecisionPolicy, DecisionClass, OwnerInvolvement, check_proposal
│   │   ├── guard.rs              Budget, PauseInfo, budget_status, llm_dispatch_allowed
│   │   ├── reconciler.rs         Drift reconciler (opinion, worktree, archive passes)
│   │   └── triage.rs             Deterministic external-request classification
│   ├── runtime/                  Agent execution runtime
│   │   ├── orchestrator.rs       Orchestrator trait, CostMetering, PlanOutput, MockOrchestrator
│   │   ├── executor.rs           Activity, ActivityKind, ActivityRunner, run_side_effect
│   │   ├── context.rs            AgentContext (assembled per-actor context)
│   │   ├── directive.rs          Directive, DirectiveKind, DirectiveStrength, DirectiveStatus
│   │   ├── mental.rs             OperatingModel
│   │   ├── persona.rs            Persona
│   │   ├── channel.rs            OwnerChannel trait + NoopChannel
│   │   ├── telegram.rs           Telegram adapter + polling loop
│   │   ├── wake.rs               WakeTier enum (tier_of)
│   │   └── watchdog.rs           Liveness watchdog
│   ├── llm/                      D2 — LLM provider client + orchestrator
│   │   ├── config.rs             ProviderConfig (base_url, api_key, model, provider)
│   │   ├── client.rs             Client seam — OpenAiClient (chat/completions) + AnthropicClient (messages)
│   │   ├── pricing.rs            PricingResolver — per-model prices (override + models.dev) + metering
│   │   ├── orchestrator.rs       LlmOrchestrator — implements Orchestrator
│   │   ├── routing.rs            ModelResolver — per-consultant model routing
│   │   └── advisor.rs            Advisor reply/summarize endpoints
│   ├── workspace/                Project boundary, git, secrets, setup
│   │   ├── project.rs            Workspace, Selfhost, ProvisionedWorktree, git_command
│   │   ├── git_observer.rs       GitObserver — branch/commit/merge → events
│   │   ├── auth.rs               Bearer token verification (constant-time)
│   │   ├── cast.rs               Roster reconcile — active-cast/ IS the roster
│   │   ├── setup.rs              SetupSpec, SetupPlan — project init engine
│   │   ├── secrets.rs            SecretStore (per-project, on-disk, never in events)
│   │   ├── provenance.rs         Provenance tracing
│   │   └── repo_metrics.rs       Repo metrics capture (tokei-based)
│   ├── consultants/              Consultant package registry
│   │   ├── cast_role.rs          CastRole enum (7 variants), ALL_CAST_ROLES
│   │   ├── loader.rs             ConsultantRegistry::from_embedded, overlay_dir
│   │   └── mod.rs                ConsultantConfig, ConsultantRegistry, RoutingConfig
│   └── web/                      Axum web server + API routes
│       ├── routes/
│       │   ├── mod.rs            Router assembly, require_auth middleware
│       │   ├── auth.rs           Login handler
│       │   ├── state.rs          GET /api/state, /api/events, SSE streaming
│       │   ├── inbox.rs          Owner inbox
│       │   ├── intake.rs         Message, brief, request, diagram handlers
│       │   ├── owner.rs          Decision, budget, pause, resume, directive, hire
│       │   ├── advisor.rs        Advisor message/handoff/summarize
│       │   ├── setup.rs          POST /api/setup, GET /api/setup/status
│       │   ├── telegram.rs       Telegram configure/status
│       │   ├── provenance.rs     Provenance queries
│       │   └── views.rs          Consultants, context, graph, model views
│       └── web.rs                Thin facade → routes::router
```

---

## 3. Core Data Model

### 3.1 Event (`src/event/mod.rs`)

The foundational type. Immutable, append-only.

| Field | Type | Notes |
|-------|------|-------|
| `event_id` | `Uuid` | Globally unique |
| `project_id` | `String` | Scoping key |
| `sequence` | `i64` | Monotonically increasing per project. Assigned by store on append. |
| `timestamp` | `DateTime<Utc>` | When the event was created |
| `actor` | `Actor` | `Owner` \| `Agent{id}` \| `System` |
| `event_type` | `EventType` | Large enum — see §3.2 |
| `aggregate` | `Aggregate{kind, id}` | Entity affected (e.g. `kind:"task", id:"task-7"`) |
| `data` | `serde_json::Value` | Event-type-specific payload |
| `metadata` | `Metadata` | `correlation_id`, `causation_id` (links to prior event), `agent_run_id` |

### 3.2 EventType (all ~60 variants)

**Organizational:** ProjectCreated, AgentHired, RequirementCreated/Changed, TaskCreated/Assigned/Started/Completed/Blocked/ReadyForReview/Reviewed, TaskPriorityChanged, MergeAuthorityChanged, TaskDecomposed, TaskBlockedOn

**Knowledge:** RiskRaised/Updated, AssumptionRecorded, ConstraintRecorded, OpinionRecorded/Superseded, FactRecorded

**Knowledge:** RiskRaised/Updated, AssumptionRecorded, ConstraintRecorded, OpinionRecorded/Superseded, FactRecorded

**Governance:** ProjectDirectiveCreated/Suspended/Resumed/Superseded/Expired

**Communication:** MessageSent, DecisionProposed/Made/Superseded, DecisionPolicyChanged

**Git semantic (see `docs/ADDENDUM.md` §23):** BranchCreated, CommitObserved, MergeCompleted, MergeConflictDetected (no producer wired yet — see §12.1), ChangeSetReady

**Worktree lifecycle:** WorktreeProvisioned, CommitRequested, WorktreeRemoved, WorktreeBound, WorktreeReleased

**External context:** AdvisoryBriefingImported, ExternalRequestReceived, DiagramSaved, AdvisorMessageSent, AdvisorHandoff

**Durable execution:** ActivityScheduled, ActivityCompleted, ActivityFailed

**Harness guards:** BudgetSet, WorkPaused, WorkResumed

**Diagnostics/audit:** PlanActionRejected, OrchestrationRun, RepoMetricsCaptured, EntityArchived

**Cost:** CostIncurred

### 3.3 Projection (`src/projection/mod.rs`)

The derived current-state — folded from the event log. NEVER authoritative.

Contains vectors of all entities: `agents`, `requirements`, `tasks`, `dependencies`, `decisions`, `messages`, `advisor_thread`, `observations`, `risks`, `assumptions`, `constraints`, `opinions`, `facts`, `spend`, `briefings`, `external_requests`, `diagrams`, `directives`, `branches`, `commits`, `merges`, `repo_metrics`, `changesets`, `worktrees`, `archived`, `rejections`, `orchestration`.

Plus derived singleton state: `budget` (Budget), `paused` (PauseInfo), `policy` (DecisionPolicy), `plan` (ProjectPlan).

Built via `Projection::build(store, project_id)` or from snapshot + tail via `store::build_from()`.

### 3.4 Projection Entity Types (`src/types/mod.rs`)

| Type | Key Fields |
|------|-----------|
| `Agent` | id, role |
| `Requirement` | id, title, description |
| `Task` | id, title, kind, status (Backlog/Working/InReview/Blocked/Done), assignee, merge_authority, priority, review, parent_id |
| `TaskStatus` | Backlog, Working, InReview, Blocked, Done |
| `MergeAuthority` | SelfMerge \| PmMerge (default) |
| `TaskReview` | reviewer, note, approved |
| `TaskDependency` | task, blocking_task, required_state |
| `Decision` | id, subject, options, recommendation, status, class, involvement, decided_by, superseded_by, owner_verdict |
| `DecisionStatus` | Proposed, Approved, Rejected, Superseded |
| `Message` | id, from, to, body |
| `Observation` | id, from, severity, subject, body, pm_action_required |
| `Risk` | id, subject, severity, status (Open/Materialized/Resolved), discovered_by |
| `Assumption` | id, body, recorded_by |
| `Constraint` | id, body, recorded_by |
| `Opinion` | id, subject, category, statement, recorded_by, status (Active/Superseded), supersedes |
| `Fact` | id, kind, statement, recorded_by, recorded_at |
| `CostEntry` | id, agent_id, task_id, cost_class, model_tier, tokens (prompt/completion/cache split), latency, estimated_usd, cost_status (actual/estimated), reported_cost_usd, incurred_at |
| `Briefing` | id, source, subject, title, body, assets, brought_in_by, status, supersedes, imported_at |
| `ExternalRequest` | id, source, external_id, title, body, reporter, labels, url, classification, severity, status, received_at |
| `Diagram` | id, title, data, saved_by, saved_at |
| `Branch` | name, task_id |
| `Commit` | sha, branch, message, author, task_id, additions, deletions, files |
| `Merge` | sha, from_branch, to_branch |
| `ChangeSet` | id, task_id, branch, commits, agent, status (Open/Ready/Merged) |
| `Worktree` | consultant, slot, task_id, branch, path, cargo_target_dir, port |
| `ActionRejection` | who, action (serialized PmAction), reason, correlation, at |
| `OrchestrationRun` | trigger, actor, correlation, context_summary, planned[], metering fields |
| `RepoMetrics` | merge_sha, captured_at, file_count, lines_by_language, coverage |
| `ArchivedRecord` | entity_kind, entity_id, summary, result, archived_at, archived_by |

### 3.5 PmAction (`src/actions/action.rs`)

The typed action vocabulary. 38 variants, serde-tagged (snake_case). Each maps to one or more domain events via `to_events()`.

**Categories** (matching the LLM-visible `ACTION_VOCAB` table):
- **ORGANISATIONAL** (PM/owner only): hire_agent, create_requirement, create_task, assign_task, set_merge_authority, decompose_task, block_task_on, apply_playbook, provision_worktree
- **TASK**: start_task, complete_task, request_review, review_task, block_task, commit_to_change_set, set_task_priority
- **DECISIONS** (PM/owner only): propose_decision, make_decision, supersede_decision, propose_consultant
- **KNOWLEDGE**: record_opinion, supersede_opinion, record_fact, record_assumption, record_constraint, raise_risk, resolve_risk, create_observation
- **GOVERNANCE** (PM/owner only): create_directive, suspend_directive, resume_directive, supersede_directive, expire_directive, propose_directive_change
- **COMMUNICATION**: send_message, import_briefing, receive_external_request, save_diagram
- **SPECIAL**: no_op
- **HARNESS GUARDS** (owner only): set_budget, pause_work, resume_work

Each `PmAction` is serializable as `{"action": "start_task", "task_id": "..."}` for LLM consumption.

### 3.6 PolicyError (`src/actions/policy.rs`)

The gate rejection vocabulary. 25 distinct error variants, each with a Display impl. Key ones:
- AgentAlreadyHired, TaskAlreadyExists/NotFound, AgentNotHired, SpecialRoleNotAssignable
- TaskNotInReview, TaskUnassigned, NotAssignee, PmMergeRequiresReview
- ActionNotAuthorized, AuthorityDowngrade, GuardAuthority, DirectiveAuthority
- DecisionNotFound/NotOpen/AlreadyOpen, RiskNotFound, OpinionNotFound
- TaskHasNoWorktree, WorktreeAlreadyProvisioned, WorktreeForOwner
- BlockedByDependency, DuplicateEntity, UnknownRole

### 3.7 Context Assembler (`src/runtime/context.rs`)

`AgentContext` — the targeted view delivered to each actor (the PM or a consultant). Contains:
- `actor` (which actor this is for)
- `objective`, `priorities`, `scored_priorities` (relevance-ranked)
- `my_tasks`, `agents` (full roster with roles), `task_assignments`
- `active_directives`, `open_risks`, `assumptions`, `constraints`
- `open_decisions`, `advisory_briefings`, `external_requests`
- `worktree` (the consultant's own isolated desk, if provisioned)

Built by `Projection::context_for(actor)`.

---

## 4. Event Sourcing Architecture

### 4.1 EventStore Trait (`src/store/event_store.rs`)

```rust
pub trait EventStore: Send + Sync {
    fn append(&self, event: Event) -> Result<Event>;  // assigns sequence
    fn read_since(&self, project_id: &str, after: i64) -> Result<Vec<Event>>;
    fn latest_sequence(&self, project_id: &str) -> Result<i64>;
    fn list_projects(&self) -> Result<Vec<String>>;
}
```

### 4.2 SQLite Backend (`src/store/sqlite_store.rs`)
- Three files in `~/.casting/<slug>/` (the project's state dir, outside the repo): `events.db`, `cursors.db`, `snapshots.db` — one per store (not a single `events.db`)
- WAL mode for concurrent read/write
- Exclusive locking mode prevents a second process from operating on the same database (dual-PM guard)
- Sequence assignment via `SELECT MAX(sequence)+1` inside IMMEDIATE transaction
- Schema: `events(event_id, project_id, sequence, timestamp, actor_type, actor_id, event_type, aggregate_kind, aggregate_id, data, correlation_id, causation_id, agent_run_id)`
- `UNIQUE(project_id, sequence)` constraint enforces no duplicates (not contiguity — see §4.3 for the gap discussion)

### 4.3 Postgres Backend (`src/store/postgres_store.rs`)
- Dedicated background thread with its own tokio runtime
- Connection auto-reconnect with bounded exponential backoff (500ms → 30s)
- Sequence allocation via ATOMIC `INSERT ... ON CONFLICT ... RETURNING next_seq - 1`
- Retries (up to 5) on UNIQUE_VIOLATION (concurrent allocator race)
- **Sequence gap caveat:** The retry-on-conflict pattern can burn sequence numbers on collision, creating gaps in the event stream. This is architecturally acceptable — the `cast log --verify` invariant (contiguous 1..max) is a SQLite-level guarantee, not a cross-backend contract. Postgres users accept gaps.
- All three stores (EventStore, CursorStore, SnapshotStore) in one connection

### 4.4 CursorStore Trait (`src/store/cursor.rs`)
```rust
pub trait CursorStore: Send + Sync {
    fn get(&self, project_id: &str, consumer: &str) -> Result<Cursor>;
    fn advance(&self, project_id: &str, consumer: &str, to: i64) -> Result<()>;
}
```
Cursors track per-consumer positions in the event stream. Consumers: "mei" (the PM), "git-observer", "reconciler".

### 4.5 SnapshotStore Trait (`src/store/snapshot.rs`)
- Snapshot = serialized Projection + the sequence it was folded through
- Disposable: on deserialization failure, falls back to full fold
- Format-versioned (`_format_version` field in JSON wrapper) for migration
- `build_from(store, snapshots, project_id)` — the canonical read path: loads snapshot, applies tail events, returns (projection, folded_through_sequence)

### 4.6 Backend Selection (`src/store/backend.rs`)
```rust
pub enum Backend {
    Sqlite { events, cursors, snapshots },
    Postgres { pg },
}
```
`from_selector("sqlite" | "postgres://...")` dispatches at startup. The three trait objects (EventStore, CursorStore, SnapshotStore) are extracted from whichever backend.

### 4.7 Write-Time Integrity (`src/event/integrity.rs`)
Opt-in (`AppState::with_integrity()`). Before an append, checks preconditions:
- `Task{Started|Blocked|Completed|ReadyForReview|Reviewed|Assigned}` → requires prior `TaskCreated` for same aggregate id
- `Decision{Made|Superseded}` → requires prior `DecisionProposed`
- Uses the current projection (which reflects prior events) for the check

### 4.8 Offline Verification (`src/event/replay.rs`)
`cast log --verify` reads the entire stream and checks:
1. Sequence is contiguous 1..max (no gaps, no dups) — **SQLite-only guarantee.** Postgres backends may have gaps due to the retry-on-conflict allocation pattern.
2. Every `DecisionMade` has a prior `DecisionProposed` for the same aggregate
3. Every `TaskCompleted` has a prior `TaskCreated`

---

## 5. The PM Control Loop

### 5.1 Architecture

```
[External events: web API / git observer / Telegram]
        │
        ▼
  AppState::append()      ← single write path: integrity gate + broadcast
        │
        ├── (opt-in) integrity::check_append() — event-level preconditions
        └── EventStore.append() — persists + assigns sequence
        │
        ▼
  broadcast::Sender.send(event)    ← wake hint (NOT the authority)
        │
        ▼
  run_pm() loop (tokio task)
    ├── Wait on broadcast (500ms timeout = quiet window)
    ├── Check WakeTier: skip Batch-tier events -> defer
    ├── Observe git state (observer runs first, through state.append)
    ├── drain()
    │   ├── Read cursor → read_since(cursor) → events
    │   ├── Projection::build (or snapshot + tail)
    │   ├── respond() — Phase 1: PM triggers
    │   │   ├── For each owner **DecisionMade**: Orchestrator::plan (expensive, smart model)
    │   │   │   ├── Guard check (budget + pause)
    │   │   │   ├── Orchestrator::plan(context, cause) → PlanOutput
    │   │   │   ├── Record OrchestrationRun audit event
    │   │   │   ├── Record CostIncurred (if metered)
    │   │   │   └── run_planned() — validate + execute
    │   │   └── For each owner **MessageSent**: DETERMINISTIC bypass (no LLM call)
    │   │       ├── CreateTask + AssignTask("mei") + ApplyPlaybook("mei/chat-interface")
    │   │       ├── insert_worktree_provisions() + expand_playbooks()
    │   │       └── run_planned() — validate + execute
    │   ├── respond() — Phase 2: Actor turns (if orchestrator present)
    │   │   └── For each actor with work (multi-pass, max 10 iters):
    │   │       └── orchestrate → run_planned
    │   └── Advance cursor to last_processed (NOT latest_sequence)
    └── Run reconciler if due (every N events)
```

**Write path distinction:** External writers (owner HTTP, Telegram, git observer) go through `AppState::append()` which runs the opt-in integrity gate but NOT the PM's business-level policy gate (`validate()` in `actions/policy.rs`). The PM's `run_planned()` path is the ONLY path through `validate()` + `to_events()` + append. This is by design: the owner is a trusted actor who can directly author events. The PM gate guards the LLM/agent-produced actions.

### 5.2 AppState (`src/pm/control.rs`)

The shared runtime state — a large struct of `Arc`-wrapped components:

| Field | Type | Purpose |
|-------|------|---------|
| `store` | `Arc<dyn EventStore>` | Event log |
| `cursors` | `Arc<dyn CursorStore>` | Consumer positions |
| `project` | `String` | Project ID |
| `snapshots` | `Option<Arc<dyn SnapshotStore>>` | Optional snapshot optimization |
| `orchestrator` | `Option<Arc<dyn Orchestrator>>` | D2 seam — None = inert |
| `auth_token` | `Option<Arc<str>>` | Owner bearer token |
| `workspace` | `Option<Arc<Workspace>>` | Real filesystem workspace |
| `secrets` | `Option<Arc<SecretStore>>` | Per-project secrets |
| `consultants` | `Arc<ConsultantRegistry>` | Consultant config registry |
| `channel` | `Arc<dyn OwnerChannel>` | Owner messaging (Telegram) |
| `reconcile_interval` | `u64` | Events between reconciler passes |
| `reconcile_passes` | `Vec<Arc<dyn ReconcilePass>>` | Pluggable reconciliation |
| `enforce_integrity` | `bool` | Write-time precondition checks |
| `decompose` | `bool` | Feature-mode parallel decomposition |
| `http_client` | `Option<reqwest::Client>` | Shared HTTP client for LLM |
| `events` | `broadcast::Sender<Event>` | In-process wake notifications |

### 5.3 Wake Tiers (`src/runtime/wake.rs`)

Three tiers control when the PM's ACT path triggers:
- **Tier-0 (Interrupt):** `MessageSent`, `DecisionMade` from owner — immediate act
- **Tier-1 (Prompt):** `Activity*`, `Task*`, `BudgetSet` — act
- **Tier-2 (Batch):** `OrchestrationRun`, `PlanActionRejected`, `CostIncurred` — defer; accumulated events fire when a non-batch event arrives or the quiet window (500ms) elapses

### 5.4 run_planned (`src/pm/control.rs:769-909`)

The core execution engine for a plan:
1. Load dedup set: all domain events since PM's cursor keyed by `(event_type_debug, aggregate_id, correlation_id)`
2. For each `(who, action)`:
   - Skip `NoOp`
   - `actions::validate(action, who, &projection)` → policy gate
   - If rejected: record `PlanActionRejected` audit event, continue
   - `action.to_events(project, who, cause, correlation)` → domain events
   - Idempotency guard: skip if already applied (same key in dedup set)
   - `state.append(event)` → store + broadcast
   - `projection.apply(&event)` — running projection
   - **Event-driven side effects:** `workspace_activity_for(&event)` → `run_side_effect` (worktree provision/commit)
   - **Write-time worktree teardown:** on TaskCompleted/ChangeSetReady/MergeCompleted → prune worktrees
   - Step delay (220ms default) for UI animation

---

## 6. Actor Turns & Consultant Orchestration

### 6.1 Per-Actor Turn Model

Phase 2 of `respond()` only runs when an orchestrator is attached. It:

1. Calls `actors_with_work(&projection)` — returns non-owner, non-done actor ids with tasks, plus the PM actor ("mei") if any task is InReview OR the PM has self-assigned tasks (via `chat-interface` playbook)
2. For each actor, assembles an `AgentContext` scoped to that actor
3. Calls `orchestrator.plan(&context, cause)` — the actor plans their own turn
4. Applies the worktree elaborator (`insert_worktree_provisions`) to insert `ProvisionWorktree` actions before `StartTask` when needed
5. Applies the playbook elaborator (`expand_playbooks`) to expand any `ApplyPlaybook` actions into child task DAGs
6. Runs through `run_planned` (policy gate + append)
7. Repeats until no actor has further work OR 10 iterations (infinite-loop safety)

### 6.2 Playbook Step Execution Mode

When an actor's context contains an `active_step` (they are executing a playbook step), the `LlmOrchestrator` switches to a **narrow execution mode**:

- **System prompt:** for consultant-owned steps, a focused instruction telling the model only to perform work relevant to this step. For PM-owned playbook steps (`chat-interface`), the full PM action vocabulary is included so the model can choose between direct work and escalation.
- **User payload:** the step contract + artifact file contents + (for PM steps) the original owner request. NOT the full company context — this is the cost win: the budget model doesn't see the entire projection.
- **Model tier:** resolved from the step's declared `CostTier` (`budget`/`standard`/`premium`), not the consultant's primary model. This is how a "premium architect" runs survey/ground passes on DeepSeek.

The step model returns `PmAction`s (either work actions like `commit_to_change_set`/`complete_task`, or — for PM steps — organisational actions like `create_task`/`assign_task` for escalation). These go through the same policy gate and `run_planned` path.

### 6.3 Worktree Elaborator (`src/pm/planning.rs`)

A deterministic rewriter that runs on every plan (both scripted and LLM-produced). Before each `StartTask`, it inserts a `ProvisionWorktree` action for the task's assignee — assigning a free port and slot from the worktree pool. This is the platform's structural isolation guarantee: the agent never needs to reason about workspace setup.

### 6.3 actors_with_work (`src/pm/planning.rs:156-189`)

Returns actors in deterministic order: iterates tasks, collects non-done, non-owner assignees who are actually hired (or are the PM). Appends the PM actor ("mei") if any task is `InReview` OR the PM has any self-assigned non-done tasks (PM tasks from the `chat-interface` playbook). Previously the PM was excluded from actor turns — now the PM gets their own actor turn when running a chat-interface playbook step, allowing a single budget model call to either implement the change or escalate. The PM is resolved by ROLE, not a hardcoded id.

---

## 7. Governance Layer (Directives)

### 7.1 Types (`src/runtime/directive.rs`)

**DirectiveKind:** Policy, Constraint, Principle, Practice, Preference, Objective
**DirectiveStrength:** Recommended < Strong < Required
**DirectiveStatus:** Active, Suspended, Superseded, Expired

**Directive** struct: id, kind, statement, scope (Vec<String>), strength, status, created_by, supersedes

### 7.2 Lifecycle

Created via `ProjectDirectiveCreated` event. Transitioned by:
- `ProjectDirectiveSuspended` → Suspended
- `ProjectDirectiveResumed` → Active
- `ProjectDirectiveSuperseded` → Superseded
- `ProjectDirectiveExpired` → Expired

### 7.3 Authority

Only the **owner** may create/change directives (enforced by `check_directive_authority` in the policy gate). PM/agents may *propose* governance changes via `ProposeDirectiveChange` (which routes as an Ask-class decision to the owner).

### 7.4 Relevance Filtering

The context assembler filters directives by governance scope — a directive only surfaces to actors whose role's scope matches.

---

## 8. Decision Policy Engine

### 8.1 DecisionClass (`src/pm/policy.rs:70-99`)

13 stable classes: InternalRename, InternalRefactor, TestingLibrary, AddConsultant, InternalImplementation, Database, Architecture, ProductRequirement, SpendingThreshold, ProductionDeployment, SecurityCritical, Irreversible, GovernanceChange

### 8.2 OwnerInvolvement (autonomy spectrum)

```
Never < Pm < Notify < Ask
```

- **Never:** organization acts without informing owner
- **Pm:** PM decides, owner not asked
- **Notify:** owner informed, work proceeds
- **Ask:** owner must decide; work is blocked

### 8.3 DecisionPolicy

Default builtin table (`builtin_involvement`):
- InternalRename/Refactor → Never
- TestingLibrary/AddConsultant/InternalImplementation → Pm
- Database/Architecture/ProductRequirement/SpendingThreshold/ProductionDeployment/Irreversible/GovernanceChange → Ask
- SecurityCritical → Notify

Overridable via `DecisionPolicyChanged` events (event-sourced). Resolution order: explicit override → builtin → `default_involvement` (Ask).

### 8.4 Authority-Downgrade Guard

`check_proposal(class, claimed_involvement, policy)` rejects any proposal that claims *less* restrictive involvement than the policy requires for its class. Since `OwnerInvolvement` is ordered (Never < Pm < Notify < Ask), the claim must be `>= required`. This prevents an LLM from silently bypassing the human by under-claiming.

---

## 9. Execution & Side Effects

### 9.1 Activity Model (`src/runtime/executor.rs`)

**ActivityKind:** LlmCall{prompt}, GitPush{branch}, Shell{cmd}, ProvisionWorktree, CommitWorktree, Inline

**Activity** struct: id (idempotency key, e.g. `task-7-llm-call-3`), target_id, kind

**ActivityRunner** trait: `fn run(&self, activity: &Activity) -> Result<ActivityResult>`

**ActivityResult:** result_ref (optional path/object id)

### 9.2 Durable Execution Protocol

1. Append `ActivityScheduled` event (intent recorded)
2. Execute side effect
3. On success: append `ActivityCompleted`; on failure: append `ActivityFailed`
4. **Idempotency:** before executing, check if `ActivityCompleted` already exists for this id — if so, skip

### 9.3 workspace_activity_for

A deterministic mapper that converts certain appended domain events into activities:
- `WorktreeProvisioned` → ActivityKind::ProvisionWorktree
- `CommitRequested` → ActivityKind::CommitWorktree
- Returns `None` for all other event types

### 9.4 run_side_effect

Shared executor that:
1. Checks `SecretStore::ensure_no_raw_secrets` (fail-closed: refuse if activity embeds a raw secret value)
2. Calls the runner
3. Appends `ActivityCompleted` or `ActivityFailed`

### 9.5 WorkspaceRunner

A concrete `ActivityRunner` that operates on the real git workspace:
- `ProvisionWorktree` → `Workspace::provision_persistent_worktree()`
- `CommitWorktree` → `Workspace::commit_in_worktree()`

### 9.6 Workspace-side-effect Failure Handling (run_planned, pm/control.rs:842-881)

When a `WorktreeProvisioned` event's physical side effect fails (git worktree add fails), the system:
1. Appends a `WorktreeRemoved` marker event (aggregate kind "worktree", id `worktree-{task_id}`, cause "provision-failed")
2. This removes the worktree from the projection
3. The StartTask gate sees no worktree → fails closed → the action is blocked

This ensures the projection aligns with physical reality.

---

## 10. LLM Integration (D2 Seam)

### 10.1 Orchestrator Trait (`src/runtime/orchestrator.rs`)

```rust
#[async_trait]
pub trait Orchestrator: Send + Sync {
    async fn plan(&self, context: &AgentContext, cause: &Event) -> Result<PlanOutput>;
}

struct PlanOutput {
    actions: Vec<PlannedAction>,     // (who, PmAction)
    metering: Option<CostMetering>,
}
```

### 10.2 LlmOrchestrator (`src/llm/orchestrator.rs`)

The real provider implementation:
1. Builds a system prompt from consultant persona + action vocabulary
2. Sends `AgentContext` as the user message
3. Calls the configured provider through the `Client` seam (see 10.6)
4. Parses the response JSON into `PmAction`s
5. Returns with `CostMetering` (see 10.7)

Supports per-consultant model routing via `ModelResolver`:
- Base config from env (provider, base_url, api_key, model)
- Per-consultant overrides via `ModelConfig` in consultant TOML files
- Ordered model chains with fallbacks

### 10.3 MockOrchestrator

A deterministic stand-in that:
- On owner `DecisionMade` with `approved=true`: creates a follow-up task
- On owner `DecisionMade` with `approved=false`: acknowledges
- Otherwise returns empty plan

Enables end-to-end testing with zero LLM and zero spend.

### 10.4 Advisor Module (`src/llm/advisor.rs`)

Owner's private strategic conversation. The advisor is an LLM-powered thinking partner, isolated from the PM's context. Only reaches the PM via explicit `AdvisorHandoff` (which produces an `AdvisoryBriefing`).

Functions: `advisor_reply`, `advisor_summarize`, `advisor_summarize_deterministic`.

### 10.5 Provider Config (`src/llm/config.rs`)

`ProviderConfig` resolves from: env vars → `~/.casting/<slug>/config.json` → inline spec. Fields: `provider` (one of `openrouter` | `openai` | `anthropic` | `litellm`), `base_url`, `api_key`, `model`.

The **persisted** config (`RuntimeConfig`) now carries `provider` and `model` alongside `api_key`, chosen in the setup wizard (`src/workspace/setup.rs` + `frontend/src/SetupWizard.tsx`). On boot `from_env(state_dir)` reads provider/model from env override or the persisted config, defaulting to `openrouter` + `deepseek/deepseek-v4-flash-0731`.

`default_base_url(provider)`: openrouter → `…/api/v1`, openai → `…/v1`, anthropic → `https://api.anthropic.com` (no `/v1` — the Anthropic client appends `/v1/messages`), litellm → localhost:4000.

`from_env(state_dir)` returns `Option<ProviderConfig>` — None when unconfigured (the default).

### 10.6 Provider Seam (`src/llm/client.rs`)

`Client` dispatches a request to whichever wire protocol the configured provider speaks, returning a normalized `ChatCompletion`/`Usage` so the orchestrator and advisor never branch on provider:

| Provider | Client | Protocol |
|----------|--------|----------|
| openrouter, openai, litellm, vllm, ollama | `OpenAiClient` | `POST /v1/chat/completions` |
| anthropic | `AnthropicClient` | `POST /v1/messages` |

The Anthropic adapter is a genuine second protocol (not a config switch): `x-api-key` + `anthropic-version` headers, system messages lifted into a top-level `system` field, `max_tokens` REQUIRED (default 8192), and response `usage` normalized from Anthropic's disjoint buckets (`input_tokens` + `cache_read_input_tokens` + `cache_creation_input_tokens`) into the shared shape, so metering sees the same fields as OpenRouter.

### 10.7 Cost Metering (`src/llm/pricing.rs`)

`CostMetering` (built by `pricing::metering`) records per-call spend with a provenance-aware cost figure and a status:

1. **Actual wins** — if the provider reports exact USD cost (`usage.cost`, which OpenRouter returns in every response), `estimated_usd` = that value and `cost_status = "actual"`. OpenAI/Anthropic direct APIs do NOT report cost → `cost_status = "estimated"`.
2. **Else estimate** — `uncached_input×P_in + cache_read×P_cache_read + cache_write×P_cache_write + output×P_out`. Cache rates are applied only when known; otherwise cache tokens are lumped at the input rate (conservative).
3. `CostMetering` also carries `reported_cost_usd`, `cost_status`, and the cache prices; all land in the `CostIncurred` event.

**Price table** — resolved by `PricingResolver` (precedence):
1. `~/.casting/<slug>/prices.json` override — `{ "provider/model": { "input", "output", "cache_read"?, "cache_write"? } }` (USD per 1M tokens). The "config, not code" escape hatch to pin/correct a price or cover a local model.
2. The cached **models.dev** dataset (`~/.casting/<slug>/models_dev_cache.json`) — the same open, key-free source Hermes uses (`https://models.dev/api.json`), covering OpenAI/Anthropic + 100s of providers with real rates. Auto-populated: `fetch_models_dev()` runs at boot behind a 24h TTL (skips when the cache is fresh); failures log and fall back.
3. Cost-tier fallback (`tier_prices` in `routing.rs`).

So the price table is auto-populated and provider-agnostic — no hardcoded per-model map in code.

---

## 11. Web API Surface

All API endpoints served by a single Axum server on `cast run`.

### 11.1 Public (Read) Endpoints

| Route | Method | Handler |
|-------|--------|---------|
| `/api/state` | GET | Full Projection JSON |
| `/api/events` | GET | All events since sequence |
| `/api/events/stream` | GET | SSE realtime stream |
| `/api/health` | GET | Liveness check |
| `/api/setup/status` | GET | Setup wizard status |
| `/api/login` | POST | Token verification |
| `/api/consultants` | GET | Consultant registry |
| `/api/context/:actor` | GET | Agent context |
| `/api/context/full` | GET | Full context |
| `/api/graph` | GET | Task graph view |
| `/api/graph/task/:id` | GET | Task context |
| `/api/model` | GET | LLM config status |
| `/api/persona/:actor` | GET | Actor persona |
| `/api/routing` | GET | Routing info |
| `/api/provenance/task/:id` | GET | Task provenance |
| `/api/provenance/decision/:id` | GET | Decision provenance |
| `/api/provenance/commit/:sha` | GET | Commit provenance |
| `/api/telegram/status` | GET | Telegram bot status |

### 11.2 Owner-Mutating Endpoints (bearer-guarded)

| Route | Method | Handler |
|-------|--------|---------|
| `/api/message` | POST | Send owner message |
| `/api/brief` | POST | Import advisor briefing |
| `/api/request` | POST | Receive external request |
| `/api/diagram` | POST | Save diagram |
| `/api/advisor/message` | POST | Advisor thread message |
| `/api/advisor/handoff` | POST | Handoff advisor→PM |
| `/api/advisor/summarize` | POST | Summarize advisor thread |
| `/api/decision` | POST | Owner decides a decision |
| `/api/policy` | POST | Change decision policy |
| `/api/directive` | POST | Create directive |
| `/api/hire` | POST | Hire agent |
| `/api/budget` | POST | Set budget |
| `/api/pause` | POST | Pause work |
| `/api/resume` | POST | Resume work |
| `/api/setup` | POST | Initial setup |
| `/api/telegram/configure` | POST | Configure Telegram bot |

### 11.3 Auth Model

- `AppState.auth_token` is `Option<Arc<str>>`. None → auth disabled.
- `require_auth` middleware wraps all mutating endpoints. When token is set, requires `Authorization: Bearer <token>`.
- First-run setup wizard works without auth (token is set during setup).
- Constant-time comparison for bearer token.

---

## 12. Git Integration & Observability

### 12.1 Git Observer (`src/workspace/git_observer.rs`)

A polling observer that runs inside the PM loop (before `drain`). On each tick:
1. Checks debounce (default 5s, configurable via `CAST_GIT_DEBOUNCE_MS`)
2. Lists all branches in the artifact repo
3. For each branch under the `casting/` prefix, lists commits since last observed
4. Emits `BranchCreated`, `CommitObserved`, and `MergeCompleted` events
5. Advancing own cursor (`"git-observer"`)

Does NOT emit: `MergeConflictDetected` (requires active merge attempt — emitted by the git runner, not the passive observer).

### 12.2 Git Runner Pinning (`src/workspace/project.rs`)

All git operations go through a single pinned command builder:
- `Workspace::git_command()` — sets `-C <repo>`, `GIT_WORK_TREE`, `GIT_DIR`
- `Workspace::git_command_for(worktree)` — same but for a worktree path
- No raw `git` call is ever exposed to agent code

### 12.3 Worktree Isolation

Two modes:
1. **Per-task worktrees** — created per task id, destroyed when the task completes/merges
2. **Persistent worktrees** — per consultant per slot, reused across tasks (build target stays warm)

---

## 13. Workspace & Project Boundary

### 13.1 Workspace (`src/workspace/project.rs`)

```rust
pub struct Workspace {
    pub repo: PathBuf,       // canonical absolute path to the artifact repo
    pub state_dir: PathBuf,  // canonical path to ~/.casting/<slug>/
    selfhost: Selfhost,
}
```

**Self-hosting guard:** `Selfhost::Disabled` refuses to operate on the Casting source repo (checks embedded source root + `name = "casting"` in Cargo.toml). Requires explicit `--selfhost` flag or `CAST_SELFHOST=1`.

**State location (external, never collocated):** All Casting internal state lives in `~/.casting/<slug>/` — a directory OUTSIDE the artifact repo (the slug is assigned at `cast init`). This keeps the user's repo byte-identical: Casting never writes a `.casting/` directory (or any state) into it. `Workspace::open` hard-refuses any state dir that lives inside the repo (`ensure_outside_repo`). Multi-project support is just N such directories under `~/.casting/`, each with its own database and port; a single `cast run` operates on exactly one.

### 13.2 Setup Engine (`src/workspace/setup.rs`)

`SetupPlan` writes the initial event sequence: `ProjectCreated`, `AgentHired` for default cast members, initial directives. Idempotent: re-running never double-hires.

### 13.3 Path Safety

`Workspace::resolve_under(requested)` ensures agent-supplied paths never escape the artifact repo. Rejects absolute paths and `..` components that would escape.

---

## 14. Consultant Registry & Roles

### 14.1 CastRole Enum (`src/consultants/cast_role.rs`)

The authoritative 7 roles: **ProjectManager**, **Advisor**, **LeadDeveloper**, **TestingEngineer**, **SystemsArchitect**, **StageManager**, **Critic**

Two are **special (non-assignable):** Advisor (and Advisor alone) orchestrate/advise and are never assigned task work. The PM was historically non-assignable but is now a carve-out: the PM may self-assign tasks via the `chat-interface` playbook for small direct work (§14.6, §19.9). The remaining five (LeadDeveloper, TestingEngineer, SystemsArchitect, StageManager, Critic) are assignable.

Each role carries: `role_id` (stable string), `title` (human), `scope` (governance area), `is_assignable()`, `is_special()`.

### 14.2 ConsultantConfig (`src/consultants/mod.rs`)

A normalized consultant package with: id, name, title, cast_role, avatar, summary, system_prompt (loaded from file), routing (specializations + trigger_patterns + auto_join), models (ordered fallback chain), assignable, max_concurrent, verification config.

### 14.3 ConsultantRegistry

Two-key registry (by_id + by_role). Supports:
- `from_embedded()` — loads curated defaults built into the binary
- `overlay_dir()` — loads user TOML files from `~/.casting/<slug>/consultants/`
- `validate_all_roles_present()` — ensures all 7 CastRole variants are bound
- `specialists_for(description)` — keyword-matching for PM routing (advisory, not authoritative)

### 14.4 Role Catalog (`src/workspace/cast.rs`)

The old hardcoded `Role` / `LEGACY_ROLES` / `role_catalog` / `role_by_id` /
`role_by_title` / `DEFAULT_CAST` tables and the `role-N` multi-instance counter
have been **deleted** (2026-08-17). **`active-cast/` IS the roster**: the single
source of who exists is `ConsultantRegistry` (loaded from the directory), and
every role is a `CastRole`-derived role declared by a consultant package —
exactly one consultant per role, no counters, no legacy ids.

Hiring is reconcile-driven: on boot and on each reconciler cadence,
`CastReconcilePass` diffs the directory against the projection's hired agents
and emits `AgentHired` for anyone present-not-yet-hired and `AgentRemoved` for
anyone hired whose package is gone. Add/remove/rename a package in
`active-cast/` and the roster follows automatically — no name hardcoding
anywhere.

The roster roles a director can see/hire come from
`ConsultantRegistry::known_roles()` (registry-derived). `POST /api/hire`
maps a role to the ONE consultant bound to it.

### 14.5 Playbooks (`src/consultants/playbook.rs`)

A playbook is a **named recipe** a consultant offers for a problem class — reusable, cost-banded step sequences that replace LLM-freeform reasoning with a structured, deterministic task DAG.

**Conceptual model:**
```
Consultant  = who (identity, role, default models, voice)
Playbook    = how (named recipe for a problem class)
Step        = one child task with a contract + a model tier
Artifact    = the only thing the next step is allowed to read
```

**TOML shape** (consultant package, `active-cast/*.toml`):
```toml
[[consultant.playbooks]]
id        = "infra-review-deep"
version   = 1
title     = "Layered infrastructure review"
problem   = "architecture-review"
summary   = "Cheap survey → expensive critique → cheap grounding."
cost_class = "architecture"
cost_band = "expensive"          # cheap | medium | expensive

[[consultant.playbooks.steps]]
id       = "survey"
title    = "Write ARCHITECTURE.md from the tree"
model    = "budget"              # CostTier, not raw model id
prompt   = """..."""
artifact = "ARCHITECTURE.md"
produces = "survey"
```

**Properties:**
- Shared parent worktree: playbook children reuse the parent's worktree. Artifacts are files on that tree. No per-step worktree.
- Same problem, several prices: a consultant offers multiple playbooks for one `problem`. The interesting difference is price/cost-band.
- No cross-role steps: the offering consultant owns every step on their worktree.
- Cost band → owner involvement: `Cheap/Medium` are PM-fireable; `Expensive` requires Ask. Three dedicated `DecisionClass` variants (`PlaybookCheap/Medium/Expensive`) provide autonomy knobs.

**Runtime model — compile, don't interpret:**
1. The PM emits `ApplyPlaybook { playbook_id, parent_task_id }` as a PmAction.
2. The **playbook elaborator** (`expand_playbooks` in `src/pm/planning.rs`) rewrites this into a `DecomposeTask` + `BlockTaskOn` chain + `AssignTask` + `ProvisionWorktree` — all deterministic, no second workflow engine.
3. Child tasks are first-class tasks on the board, visible in the UI, crash-recoverable, and go through the standard actor-turn lifecycle.
4. The **step execution mode** in `LlmOrchestrator` narrows the context payload to just the step contract + artifact contents (not the full company state) and routes to the step's declared `CostTier` model.

**Three dispatch paths:**
1. **Packaged playbook** — existing TOML recipe. Cheapest and safest path.
2. **Sink** — no matching playbook → assign to LeadDeveloper on budget model. Always available.
3. **Ad-hoc recipe** — PM may emit a one-off `AdHocRecipe` inline. Same elaborator, same artifact gates, recorded as `PlaybookApplied { source: ad_hoc }`.

### 14.6 Chat-Interface Playbook (PM Quick-Change Path)

The PM (mei) offers a special `chat-interface` playbook that implements a **no-orchestrator bypass** for owner chat requests (§5.1 flow diagram, §19.9). This is the system's answer to "rename a string" — the most common case where routing overhead would exceed change cost.

**Flow:**
```
Owner sends "rename X to Y"  (MessageSent event)
  │
  ▼
Phase 1 — respond() ── DETERMINISTIC (no LLM call) ──► CreateTask("chat-xxx")
                                                         AssignTask("chat-xxx", "mei")
                                                         ApplyPlaybook("mei/chat-interface")
                                                         expand_playbooks() → child step + worktree
  │
  ▼
Phase 2 — run_actor_turns()
  │
  ▼
PM actor turn (budget model) ← FIRST AND ONLY LLM CALL
  ├── Path A (direct): commit_to_change_set + complete_task(x2)
  │   └── Change applied, task done. No routing, no handoff.
  └── Path B (escalate): create_task + assign_task(to Diego) + complete_task(x2)
      └── Diego picks it up next loop iteration.
```

**Why this works:**
- Owner messages are the only trigger type that bypasses the orchestrator. Owner decisions (`DecisionMade`) still route through the full smart-model PM orchestrator.
- The PM persona was updated from "you coordinate, you do not implement" to "for tiny safe changes, apply the chat-interface playbook and do it yourself."
- The playbook's step model tier is `budget` (cheapest) so the first LLM call is always the cheapest possible.
- The step's action vocab is the **full PM vocabulary** (including `CreateTask`, `AssignTask`, `ApplyPlaybook`) — so the budget model can escalate by emitting organisational actions, not just work actions.

**Assignability carve-out (all role-resolved, no hardcoded ids):**
- Special-role checks now go through `is_pm_actor` / `is_advisor_actor` (by `CastRole`), not a hardcoded id.
- `is_valid_assignee()` returns `true` for the PM actor (resolved by role).
- The policy gate's `AssignTask` validation allows the PM actor as assignee (while still rejecting the Advisor).
- `ProvisionWorktree` validation also allows the PM actor as assignee (worktrees are needed for the chat step).
- `HireAgent` still rejects the PM/Advisor actors — they are built-in roles, not hirable.
- `actors_with_work` in `src/pm/planning.rs` includes the PM when they have self-assigned tasks (the PM carve-out bypasses the standard `agents.contains()` check).

### 14.7 Consultant Packages — Directory Layout & Private Asset Banks (2026-08-17)

Each consultant is its own **directory named by consultant id** (`active-cast/<id>/`),
not a single flat TOML — so a consultant can carry large reference material and
a set of private capabilities, and is the natural unit for future sharing/marketplace.

**A consultant's id is always a NAME** (e.g. `mei`, `jeeves`, `diego`) — never a
role. The PM and Advisor are the named people `mei` (Project Manager) and
`jeeves` (Advisor); the application identifies them by their **role**
(`CastRole::ProjectManager` / `CastRole::Advisor`), never by assuming an id. The
role key (`role_id()` returning `"pm"`/`"advisor"`) is a separate, stable concept.

```
active-cast/<id>/
  consultant.toml         # manifest: identity, role, models, routing, indices
  system_prompt.md        # persona (referenced via system_prompt_file)
  skills/<slice>.md       # capability/procedure slices (differentiator)
  knowledge/<slice>.md    # declarative reference slices (makes it smarter)
  playbooks/<pb>.toml     # one playbook per file ([playbook] top-level table)
```

The manifest keeps identity/role/models/routing inline and **references** the
persona + asset/playbook bodies by relative path (resolved at load time, so
large references never bloat the manifest). A legacy single-file package
(inline `system_prompt` + inline `[[consultant.playbooks]]`) still loads for
overlay backward compatibility.

**Asset banks (`skills` + `knowledge`)** — a uniform `AssetSlice` used two ways:
- **skills** = procedures — what this consultant can do that others can't. Surfaced
  to the PM for routing so it can reason over differentiation.
- **knowledge** = reference facts — language docs, API cheatsheets — that make the
  consultant smarter.

Each slice has a `char_budget`. **Playbook steps declare the exact slices they need:**

```toml
[[playbook.steps]]
id        = "survey"
...
requires_knowledge = ["casting-conventions"]
requires_skills    = ["worktree-commits"]
```

At step dispatch the step executor resolves `requires_*` ids against the owning
consultant's banks (`ConsultantConfig::required_slices`) and the orchestrator
inlines **only those exact bodies** (truncated to each slice's `char_budget`) into
the step prompt — never the whole bank. Slices carry through the existing
cost/budget seam. Loading is **fail-closed**: a step requiring a slice the
consultant doesn't own rejects the whole package.

`AgentContext.available_assets` surfaces every consultant's skill/knowledge cards
to the PM (alongside `available_playbooks`) for routing/differentiation reasoning.

---

## 15. Reconciler & Drift Management

### 15.1 Cursor-Gated Cadence

The reconciler has its own cursor (`"reconciler"`). It runs when `latest_sequence - reconciler_cursor >= reconcile_interval` (default 25 events). Runs inside the PM loop after drain.

### 15.2 Pluggable Passes (`src/pm/reconciler.rs`)

Trait: `ReconcilePass { fn name() -> &str; fn run(&state) -> Result<u32>; }`

Default passes:
1. **OpinionDriftPass:** detects two Active opinions with the same subject → supersedes the older one
2. **StaleWorktreePass:** prunes worktrees whose task is Done or ChangeSet is Merged (safety net; eager teardown also happens at write-time)
3. **ArchivePass:** archives terminal entities (done tasks, superseded decisions/opinions, resolved risks) by emitting `EntityArchived` events, removing them from the active projection to save agent context tokens

---

## 16. Harness Guards (Budget, Pause, Secrets)

### 16.1 Budget (`src/pm/guard.rs`)

- Owner-set via `BudgetSet` event: `limit_usd` + `warn_at` (default 0.80)
- `budget_status(proj)` → Disabled | Ok | Warn{fraction} | Halted{fraction}
- Derived from `proj.total_spend_usd()` which is always recomputed from `CostIncurred` events — never a side ledger
- **Halted is permanent** until a higher budget limit is set: spend never decreases, so `ResumeWork` does NOT clear a budget halt

### 16.2 Pause (`src/pm/guard.rs`)

- Owner or liveness watchdog sets via `WorkPaused` event
- Cleared via `WorkResumed` event
- Orthogonal to budget halt

### 16.3 llm_dispatch_allowed (`src/pm/guard.rs:112-124`)

The unified gate: checks both paused AND budget-halted. Called before every LLM orchestrator call. Returns `Err(reason)` when work should be blocked.

### 16.4 SecretStore (`src/workspace/secrets.rs`)

- Values stored on disk in `~/.casting/<slug>/secrets.json` (outside the repo)
- Referenced in activities via `@secret:NAME@` placeholders
- `ensure_no_raw_secrets(activity)` — fail-closed guard: refuses to schedule/execute an activity that embeds a raw secret value verbatim
- This is the one genuinely hard invariant: once a secret's raw value lands in an event (via `ActivityScheduled` with `Shell{cmd}` or `LlmCall{prompt}`), it is in the append-only log forever

---

## 17. Telegram Channel (Owner Outreach)

### 17.1 OwnerChannel Trait (`src/runtime/channel.rs`)

```rust
pub trait OwnerChannel: Send + Sync {
    fn send_message(&self, to: &str, text: &str) -> Result<()>;
}
```
Default: `NoopChannel` — a pipe to nowhere. Replaced by the Telegram adapter when configured.

### 17.2 Telegram Adapter (`src/runtime/telegram.rs`)

Polling loop: polls Telegram for new messages → emits events into the event store. Supports:
- `/start` → binds chat to owner (first-DM-is-owner binding)
- Owner messages → `MessageSent` event
- Advisor messages → `AdvisorMessageSent` event (isolated from PM context)

Configured via env (`CAST_TELEGRAM_BOT_TOKEN`, `CAST_TELEGRAM_CHAT_ID`) or via `POST /api/telegram/configure`.

---

## 18. Event Flow Diagram (Text)

```
Owner/External Trigger           PM Loop                          Git Observer
══════════════════════          ════════                         ═════════════
  │                               │                                  │
  │ POST /api/message             │                                  │
  │  └─ AppState::append()        │                                  │
  │      ├─ EventStore.append()   │                                  │
  │      └─ broadcast::send() ────┤── wake hint ────────────────────┤
  │                               │                                  │
  │                               │ run_pm() loop wakes              │
  │                               │ ├─ Check tier (Interrupt → act)  │
  │                               │ ├─ observe_once() ───────────────┤
  │                               │ │   (git observer runs)           │
  │                               │ ├─ drain()                       │
  │                               │ │   ├─ Read cursor               │
  │                               │ │   ├─ read_since(cursor)        │
  │                               │ │   ├─ Projection::build()       │
  │                               │ │   ├─ respond()                 │
  │                               │ │   │   ├─ Phase 1: PM triggers  │
  │                               │ │   │   │   └─ Orchestrator::plan │
  │                               │ │   │   │       ├─ guard check    │
  │                               │ │   │   │       ├─ context_for()  │
  │                               │ │   │   │       ├─ LLM call       │
  │                               │ │   │   │       ├─ parse PmActions│
  │                               │ │   │   │       └─ PlanOutput     │
  │                               │ │   │   └─ run_planned()         │
  │                               │ │   │       ├─ validate() gate    │
  │                               │ │   │       ├─ to_events()        │
  │                               │ │   │       ├─ append (audit)     │
  │                               │ │   │       ├─ workspace_effect   │
  │                               │ │   │       └─ write-time teardown│
  │                               │ │   │   └─ Phase 2: Actor turns   │
  │                               │ │   │       └─ (per assignee)     │
  │                               │ │   └─ Advance cursor             │
  │                               │ └─ reconciler::run_if_due()       │
  │                               │                                  │
  │                    ───────────┼── broadcast ──────────────────────│── SSE
  │                               │                                  │  clients
```

---

## 19. Key Architecturally-Significant Design Decisions

### 19.1 Event Log as Sole Authority
Nothing is authoritative except the append-only event log. Projections are re-derived on every read (or from snapshot + tail). Cursors mark position but never store state.

### 19.2 LLM as an Optional Seam, Not the Core
Without an orchestrator attached, the system is **properly inert**. Owner messages and decisions are recorded but no action is taken. The mock orchestrator enables full end-to-end testing with zero spend. The real LLM plugs in through the same `Orchestrator` trait and every action it produces goes through the same policy gate.

### 19.3 Fail-Closed Design
- `MergeAuthority` defaults to `PmMerge` (work requires review)
- `DecisionPolicy` defaults to `Ask` (owner must decide)
- `PmAction` match in `validate()` has **no wildcard arm** — every variant is enumerated
- Unassigned tasks, missing worktrees, unsatisfied dependencies — all reject with clear errors
- The authority-downgrade guard prevents LLM bypass of owner involvement

### 19.4 Dual Authority Chain
Two independent guard mechanisms:
1. **Policy gate** (actions/policy.rs) — pure, deterministic, LLM-free. Validates every proposed action against the current projection.
2. **Harness guards** (pm/guard.rs) — budget and pause rails that sit outside the PM's control. The PM can be compromised; these cannot.

### 19.5 Write-Time vs Cadence Cleanup
Critical cleanup (worktree teardown on task completion) happens eagerly at write-time in `run_planned`. The reconciler's periodic passes are a safety net, not the primary mechanism. This ensures ports are freed immediately.

### 19.6 Idempotent Drain
The PM drain deduplicates domain events it has already applied for the same planning cause, using a key of `(EventType, aggregate_id, correlation_id)`. This prevents a mid-drain failure (cursor not advanced) from re-emitting duplicate events on the next drain. Audit/telemetry events (`OrchestrationRun`, `PlanActionRejected`, `CostIncurred`, etc.) are deliberately excluded from dedup — they must keep appending.

### 19.7 Leapfrog Race Prevention
The cursor is advanced to `last_processed` (the last event the PM just processed), not `latest_sequence`. This prevents the leapfrog race: if a concurrent writer (web handler, git observer) appends after the PM's `read_since` but before its `advance`, advancing to `latest_sequence` would skip those events forever.

### 19.8 Snapshot Read-Save Race Prevention
`store::build_from` returns the exact sequence the projection was folded through. The caller saves the snapshot at THAT sequence, not a fresh `latest_sequence` read. This prevents the race where a concurrent append between the fold and the read would be marked as folded but skipped on future rebuilds.

### 19.9 Orchestrator Bypass for Chat Messages
Owner chat messages (`MessageSent`) are the highest-frequency trigger and the lowest-value LLM call — routing "rename this string" through Sonnet to decide "send it to Diego" burns expensive tokens for a trivial decision. The system solves this with a **deterministic Phase 1 bypass**: owner messages skip the Orchestrator entirely and go straight to a cheap budget-model actor turn via the `chat-interface` playbook (§5.6, §14.6). Owner decisions (`DecisionMade`) still route through the full orchestrator — those require strategic reasoning. This is a deliberate asymmetry: the bypass is not a general "messages are cheap" rule but a targeted optimization for the common trivial case. If the budget model decides the change is too complex, it escalates by emitting organisational actions (`CreateTask`, `AssignTask`) that route the work to the right consultant — all within the same cheap call.

---

## 20. Known Gaps & Improvement Areas

These are verified, open issues in the architecture — either deferred to a future milestone or requiring owner action.

### 20.1 Budget Defaults to Disabled
The budget system (§16.1) is event-driven and off by default (`Budget::default()` sets `limit_usd: 0.0`). A live LLM run with no budget set has unbounded spend. Fix: warn at startup if an orchestrator is attached without a configured budget. Owner must set a budget via `BudgetSet` event (API or `cast budget`).

### 20.2 No Structured Observability
The system has no metrics, tracing, or operations dashboards. Audit events (`OrchestrationRun`, `PlanActionRejected`, `CostIncurred`) serve as the diagnostic trail, but there is no mechanism to detect a stuck PM loop, measure LLM latency, or alert on budget exhaustion. The liveness watchdog exists (it can pause work) but its design and integration are not yet documented.

### 20.3 LLM Call Non-Determinism on Crash Recovery
The durable-execution protocol (§9.2) prevents redundant side effects via idempotency keys, but an `LlmCall` activity re-dispatched after a crash may produce *different* output than the original call. The architecture has no mechanism to pin LLM output to a nonce or enforce deterministic replay.

### 20.4 Worktree Port Allocation (Elaborator)
The worktree elaborator (§6.2) assigns free ports from a pool when rewriting a plan. Under the current single-threaded PM loop this is safe in practice, but there is no explicit reservation+commit handshake — a future parallel-actor extension would break. See the elaborator in `src/pm/planning.rs`.

### 20.5 ArchivePass Is One-Way
The reconciler's ArchivePass (§15.2) removes terminal entities from the active projection. There is no un-archive mechanism and no way to search archived entities. The event log preserves the data, but the PM acts only on the projection.

### 20.6 Secret Scrubbing Scope
The `ensure_no_raw_secrets` guard (§16.4) only checks `Activity` payloads. User/LLM-authored text in `MessageSent` bodies, `Briefing` content, `Decision` text, and `ExternalRequest` fields is not scanned. A secret value accidentally pasted into these fields becomes permanent in the append-only log. (Addressing: secret scanning should be extended to all free-text fields.)

### 20.7 Postgres as First-Class Backend
Postgres with a dedicated connection thread, auto-reconnect, and retry logic is significant complexity for a single-project, locally-collocated tool. Consider deferring it alongside multi-project support.

---

> **This document is an architectural inventory of the Casting codebase, reviewed against the source at the points listed above.**
> Every type, trait, call flow, and relationship described here has been verified against source.
> For a domain expert review: the key areas to scrutinise are the **policy gate** (authority enforcement across all paths), the **idempotency model** (dedup keys, crash recovery gaps), the **worktree elaborator** (free port race conditions), and **cost/liveness guard** defaults (disabled vs enabled at startup).