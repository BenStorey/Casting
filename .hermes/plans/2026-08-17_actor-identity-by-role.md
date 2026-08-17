# Deduplicate actor identity: stop the file structure from leaking into the app

**Date:** 2026-08-17
**Status:** in progress — Phase 1 (role-based resolution) committed & green; Phase 2 (rename ids pm→mei, advisor→jeeves) in flight: packages/dirs/seeds/data/docs updated, test sweep being updated.
**Owner:** Ben + Hermes

## Problem
Consultant ids are inconsistent: the 5 assignable consultants use a NAME as id
(ali, diego, ...) but the 2 special roles use a ROLE as id ("pm", "advisor"),
and those ids equal their `active-cast/` folder names. The application reaches
for these actors by the hardcoded id string (`by_id("pm")`, `context_for("pm")`,
`Actor::Agent { id: "pm" }`, `SPECIAL_ACTORS = ["advisor"]`, `source: "advisor"`),
so the *file/directory layout* leaks into behaviour + event identity.

Ben: the loader should own "who is who" (parse all files → one consultant per
role, validated), and the app should resolve "the PM"/"the advisor" **by role**,
never by knowing their id happens to be a folder name.

## Key distinction (drives the whole design)
- **Pseudo-actors `director` / `system`** — NOT consultant-backed. No package, no
  folder, no id-in-layout. These are legitimate DOMAIN constants, not a leak.
- **Consultant-backed actors (the PM, the Advisor)** — backed by a consultant
  package. These are currently reached by the id string that equals the folder
  name. THIS is the leak. They must be resolved **by role** instead.

## Target rule (after this change)
1. Every consultant id is a NAME (mei, jeeves, ali, diego, ...). Dir name == id.
2. The PM and Advisor are identified by **CastRole** (`CastRole::ProjectManager`,
   `CastRole::Advisor`), resolved to their person via the registry
   (`for_cast_role(role).id` or `actor_for(role)`).
3. `director` / `system` remain explicit domain constants (not consultant-backed).
4. Event seeds write the PM/Advisor by their PERSON id (mei/jeeves).

## Renaming map
| old id | new id | name | cast_role |
|--------|--------|------|-----------|
| pm      | mei    | mei  | project_manager |
| advisor | jeeves | Jeeves | advisor     |

Folder renames: `active-cast/pm/` → `active-cast/mei/`, `active-cast/advisor/` →
`active-cast/jeeves/`.

## Core mechanism
Add on `ConsultantRegistry`:
- `actor_for(&self, CastRole) -> Option<&str>` — the actor id (name) filling a
  role (delegates to `for_cast_role(role).map(|c| c.id)`).
- `actor_is(&self, who: &str, CastRole) -> bool` — `who == actor_for(role)`.
Add an `ActorRole` helper for the policy gate: given `who` + registry, classify
as Director | System | Pm (by role) | Advisor (by role, non-assignable) | HiredAgent.

Route every pm/advisor *lookup + actor category* check through these. Keep
`DIRECTOR`/`SYSTEM` constants. No code path may compare `who == "pm"` or
`who == "advisor"`.

## Execution order (safe, compile-green at each phase)
- **Phase 1 — decouple lookup to role (no behavior change):** add `actor_for`/
  `actor_is` + ActorRole; refactor ALL src lookups (`by_id("pm")`,
  `context_for("pm")`, `resolver.resolve("advisor")`, policy checks, seeds that
  hardcode id) to resolve via role. Registry still returns "pm"/"advisor" at
  runtime, so every test stays green. Proven decoupling, zero behavior delta.
- **Phase 2 — rename identity:** flip ids pm→mei, advisor→jeeves, rename dirs,
  update event seeds + data `source:` labels. Now nothing internal breaks
  because lookups are role-based. Update tests.
- **Phase 3 — docs + full suite green + commit.**

## Risk notes
- Existing companies' histories carry `Actor::Agent { id: "pm" }`; new seeds will
  write "mei". Replay records whatever ids exist, so old companies stay valid;
  the deltas are only in NEW seeds and in policy KEYED on role. No migration of
  historical events (dev, no prod state).
- `source: "advisor"` data labels: become the advisor person id (jeeves) so they
  stay consistent with actor identity.