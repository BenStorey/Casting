# Consultant packages: skills + knowledge bank + playbook file split

**Date:** 2026-08-17
**Status:** done — implemented 2026-08-17 (commit pending)
**Owner:** Ben + Hermes

## Motivation
Consultants currently ship as one flat self-contained `.toml` with inline
`system_prompt`, inline `[[playbooks]]`, and no way to give a consultant its
own private *knowledge* (reference docs that make it smarter) or *skills*
(procedures that differentiate what it can do). Cramming a kdb+/q language
reference into a single TOML is untenable.

## Decision (Ben, 2026-08-17)
Each consultant becomes its own **directory named by consultant id**, with a
fixed structure. A top-level manifest references other files where it makes
sense. Playbooks move to their own `playbooks/` dir of `.toml` files,
referenced from the manifest.

## Target layout
```
active-cast/<id>/
  consultant.toml         # manifest: identity, role, models, routing, assets index
  system_prompt.md        # persona (moved out of inline)
  skills/<slice>.md       # capability/procedure slices (differentiator)
  knowledge/<slice>.md    # declarative reference slices (stronger)
  playbooks/<pb>.toml     # each playbook as its own file ([playbook] table)
```

`consultant.toml` manifest keeps inline: id, name, title, cast_role, avatar,
summary, routing, models, verification, assignable, max_concurrent. It gains:
- `system_prompt_file = "system_prompt.md"`
- `[[consultant.skills]] id/title/char_budget/file = "skills/x.md"`
- `[[consultant.knowledge]] id/title/char_budget/file = "knowledge/x.md"`
- `[[consultant.playbooks]] file = "playbooks/x.toml"`

## Code changes
1. `src/consultants/mod.rs` — add uniform `KnowledgeSlice`/`SkillSlice` asset
   struct (`id`, `title`, `char_budget`, `body`). ConsultantConfig gains
   `skills: Vec<Slice>`, `knowledge: Vec<Slice>`.
2. `src/consultants/playbook.rs` — `PlaybookStep` gains
   `requires_skills: Vec<String>`, `requires_knowledge: Vec<String>`;
   validated non-empty + resolvable against the owning consultant's assets
   (fail-closed at load).
3. `src/consultants/loader.rs` — restructure to directory packages: enumerate
   top-level dirs under `active-cast/` (embedded) and
   `<project>/.casting/consultants/<id>/` (overlay), parse manifest, resolve
   referenced system_prompt/skills/knowledge/playbook files. Keep strict
   validation + all-7-roles check.
4. `src/runtime/context.rs` — `ActiveStepContext` carries the resolved
   required slice ids (or bodies).
5. `src/pm/control.rs` + `src/llm/orchestrator.rs` — when dispatching a
   playbook step, resolve `requires_skills`/`requires_knowledge` against the
   executing consultant's bank and inline **only those** bodies into the step
   prompt, capped by `char_budget` + a per-step total cap. No whole-bank dump.
6. `src/runtime/context.rs` — surface each consultant's skills/knowledge ids
   to the PM routing context (like `available_playbooks`) so the PM reasons
   over differentiation.
7. Migrate all 7 `active-cast/*.toml` to directory packages.
8. Tests: migrate `tests/consultants.rs` overlay tests to directories; add
   coverage for skills/knowledge resolution + step injection + fail-closed
   validation.

## Thru-cost / budget alignment
Injected slice chars count toward the same per-step context cap and
`CostMetering` seam already used by step execution — no new spend surface.