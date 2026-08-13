# Surface the Operating Picture + Provenance + Request Inbox in the SPA (Dashboard UI)

> **For Hermes:** implement task-by-task; TDD where it produces code; commit + push after each feature.

**Goal:** Two deterministic, LLM-free increments that complete the product surface:
**#1** render the already-built operating picture (`/api/model`), provenance, and the
external-request inbox in the SPA; **#2** let the owner define NEW role types
(event-sourced, not hardcoded) and hire/cast agents of them.

**Architecture:** Both stay on the existing invariants. #1 is additive frontend only
(the backend `mental.rs`/`provenance.rs` already return everything — it's just never
rendered). #2 event-sources the role catalog: `RoleDefined` events fold into
`Projection.role_catalog` (builtin seed + custom appended), and every runtime role
lookup resolves against that projection instead of the static `ROLE_CATALOG`.

**Tech Stack:** Rust (axum 0.8, event-sourced), React 19 + Vite 8 + TS 7 + Tailwind 4
+ shadcn/ui. CI on GitHub Actions.

---

## Feature #1 — Surface the Operating Picture + Provenance + Request Inbox

### Why

`GET /api/model` (the curated owner dashboard Ben built specifically *to be looked at*),
`/api/provenance/*`, and the external-request intake are all built server-side but have
**zero** references in the SPA (store only hydrates `/api/state`). This closes that gap.

### Approach

Add `fetchModel()` + typed `OperatingModel` mirror to `frontend/src/api.ts`, then a new
**Overview** tab in `App.tsx` that renders the operating picture from `/api/model`: objective,
ranked priorities, governance (active directives, decision policy, open decisions), knowledge
(opinions, superseded, facts, briefings), context (risks, requirements, task counts, agents),
the **request inbox** (open requests with classification/severity), spend, active worktrees,
per-actor contexts, and drift signals. A smaller **Provenance** panel looks up a task's chain
(`/api/provenance/task/{id}`) — "why does this code exist". The store gains a `model` slice
refreshed alongside `state`.

### Files
- Modify: `frontend/src/api.ts` (types + `fetchModel`, `fetchProvenance`)
- Modify: `frontend/src/store.ts` (model slice, refresh it on SSE)
- Modify: `frontend/src/App.tsx` (Overview tab + Provenance panel)
- Possibly new: `frontend/src/components/overview.tsx` (keep App.tsx from bloating)

### Validation
- `npm run build` in `frontend/` (embedded SPA must build before cargo build).
- `cargo build` + `make` gate green.
- The Overview tab renders the operating picture against a live `cast run`.

### Commit
`feat(ui): surface the operating picture (/api/model) + provenance + request inbox in the SPA`

---

## Feature #2 — Owner-Creatable Role Types (event-sourced cast extension)

### Why

`ROLE_CATALOG` is a hardcoded static const (engineer/qa/security/devops). "Different CEOs
build different casts" is only half true — the owner can hire, but not *invent* a role. The
skill flags owner-creating roles as "more consequential" but that consequence (a per-role model
+ prompt for D2) is deferred; the *role definition itself* (id/title/scope) is deterministic
config we can event-source now.

### Design

Keep `BUILTIN_ROLES` as a static seed (used by setup/`cast init` before a company exists and by
`plan_onboard` hiring the `DEFAULT_CAST`). Event-source the runtime catalog:

1. **`Role`** becomes owned `String` fields (was `&'static str`); `RoleDefined { id, title, scope }`
   event (actor Owner) folds into `Projection.role_catalog`:
   `role_catalog = BUILTIN_ROLES (seeded at build) + custom roles from RoleDefined events`.
2. **`Projection::role_by_id(id)` / `role_by_title(title)`** — resolve against `role_catalog`.
   Point every *runtime* call site at these (they already have a `&Projection`):
   - `context.rs scopes_for` (`role_by_title`)
   - `web/routes/owner.rs hire_handler` (`role_by_id`)
   - `actions/policy.rs ProposeConsultant` validate (`role_by_id`)
   - `web/routes/setup.rs setup_status_handler` (list `proj.role_catalog`)
   - `pm.rs` consultant-hire title resolution (`role_by_id`, fallback already fine)
3. **`POST /api/role {id, title, scope}`** on the guarded owner surface → validated `RoleDefined`.
   Validation: `id`/`title`/`scope` non-empty, `id` not already defined (builtin or custom).
4. **Frontend:** the Team view gets a "Define a new role" form; newly-defined roles flow into the
   hire/cast-picker and `/api/model` `active_agents`.

### Files
- Modify: `src/cast.rs` (owned `Role`, `BUILTIN_ROLES`, keep `DEFAULT_CAST`)
- Modify: `src/types.rs` / `src/event.rs` (`RoleDefined` event + reducer) — see exact homes while implementing
- Modify: `src/context.rs`, `src/web/routes/owner.rs`, `src/actions/policy.rs`, `src/web/routes/setup.rs`, `src/pm.rs`
- Modify: `src/web/routes/mod.rs` (mount `/api/role` on the guarded router)
- Modify: `src/web/routes/owner.rs` (role handler) + `tests/web_boot.rs` (router-boot regression)
- New tests: custom role definition → hire → `scopes_for` derives the new role's scope
- Frontend: `frontend/src/api.ts`, Team view in `App.tsx`/components

### Validation
- `cargo build --tests` to catch every role-usage site + test literal.
- New test(s) in a `cast`/`team` test file: define role → hire agent of it → context scope correct.
- `make` gate green (221+ tests), clippy 0, fmt clean.
- Update the casting skill with the new RoleDefined surface + pitfalls.

### Commit
`feat(cast): owner-creatable role types — event-sourced RoleDefined catalog + POST /api/role`

---

## Risks / tradeoffs

- #2 changes the public `Role` type (lifetimes → owned) — touches test literals; fix via
  `cargo build --tests` surfacing all at once.
- Keep `Role` serialization stable for the SPA `SetupRole` shape (`id`/`title`/`scope`).
- Do NOT touch multi-user, multi-project, real-LLM, or Telegram — explicitly out of scope.
