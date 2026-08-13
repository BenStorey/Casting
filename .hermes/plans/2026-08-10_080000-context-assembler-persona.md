# Context Assembler + Persona/CV — Implementation Plan

> **For Hermes:** implement each as a separate commit (per owner).

**Goal:** two deterministic read-side features that make the mature core
*usable* — the synthesis layer an agent (or the owner) consumes.

## Feature A — Context Assembler (docs/SEMANTIC_EVENTS §21)

The payoff of the state-core: combine projection + plan + governance + risks +
decisions + tasks into a **targeted operating context per agent/role**, instead
of handing an agent the whole event log. Pure derivation, no LLM — but this is
exactly the seam D2's orchestrator will read from.

`src/context.rs` (new):

```rust
pub struct AgentContext {
    pub role: String,                 // Principal Engineer / QA / owner / pm
    pub objective: Option<String>,    // the current plan objective
    pub priorities: Vec<PlannedItem>, // ranked current work
    pub my_tasks: Vec<String>,        // open(non-done) tasks assigned to this actor
    pub active_directives: Vec<String>,// filtered by scope via directive::relevant
    pub open_risks: Vec<String>,
    pub assumptions: Vec<String>,
    pub constraints: Vec<String>,
    pub open_decisions: Vec<String>,  // decisions awaiting the owner
}
```

- `Projection::context_for(&self, actor: &str) -> AgentContext` — assembles from
  the projection + `directive::relevant(self, areas_for(actor))` + `self.plan()`.
  - `my_tasks` = tasks with `assignee == actor` and status not Done.
  - A free `scope` union of that actor's task kinds + a per-role default (e.g.
    "engineering" for Marcus, "qa" for Maya, everything for owner/pm).
- Expose `GET /api/context/{actor}` (owner → whole-project view).

## Feature B — Persona / CV rendering (brief §2.2)

A pure renderer turning an agent's real state into a *persona* / CV card —
identity layer over the underlying configuration, never a separate source of
truth.

`src/persona.rs` (new):

```rust
pub struct Persona {
    pub id: String,
    pub role: String,
    pub status: String,          // active (hired, not fired)
    pub title: String,           // role + specialization
    pub current_tasks: Vec<String>,   // open tasks assigned
    pub completed_tasks: usize,       // count of Done
    pub highlights: Vec<String>,      // titles of recent Done tasks
    pub directives_applicable: Vec<String>, // scope-filtered
    pub updated_at: String,      // last event timestamp involving this agent
}
```

- `Projection::persona_for(actor) -> Option<Persona>` — None if the agent isn't
  hired. Derives from tasks (mine / done) + `directive::relevant`.
- Expose `GET /api/persona/{agent_id}`.

Both are pure reads over the projection — no new events, no new state.

---

## File changes

| File | Change |
|---|---|
| Create `src/context.rs` | `AgentContext` + `Projection::context_for` |
| Create `src/persona.rs` | `Persona` + `Projection::persona_for` |
| Modify `src/lib.rs` | `pub mod context; pub mod persona;` |
| Modify `src/web.rs` | `GET /api/context/{actor}`, `GET /api/persona/{id}` |
| Create `tests/context.rs` | assembly tests |
| Create `tests/persona.rs` | persona tests |

## Tasks / commits

1. **Context Assembler** — commit:
   `feat(context): assemble a per-agent operating context (state+plan+governance)`.
2. **Persona/CV** — commit:
   `feat(persona): render an agent's CV card from derived state`.
3. **Docs** — layout + roadmap + test count. Commit.

## Tests / validation

- `tests/context.rs`: Marcus context includes his tasks + engineering directives,
  excludes Maya's; objective present; owner view includes everything.
- `tests/persona.rs`: hired agent → persona with current/completed tasks +
  applicable directives; unknown agent → None; completed count increments.
- Full gate `make`. Currently 110 tests.

## Risks / tradeoffs

- **Scope resolution is a heuristic** (task-kind + role-default). It's
  deliberate and deterministic; a richer taxonomy is future hardening.
- Both are derived views — never authoritative. The event log stays the source.