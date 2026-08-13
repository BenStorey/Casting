# Home-directory project registry — multi-project ready — Implementation Plan

> **For Hermes:** implement with TDD; commit; push. Owner decisions below.

## Decisions (owner, 2026-08-10)

1. **Drop multi-user entirely.** The git repo is the collaboration surface —
   different humans run their OWN Casting setups. Single-owner auth forever
   (password / signed key); no users, roles, or permissions to build.
2. **Home-directory registry.** Instead of passing CLI params every run,
   projects live in **`~/.casting/projects.json`** (a small registry: name →
   repo path). This is the *launcher*; per-project *state* stays collocated in
   `<repo>/.casting/` (gitignored, portable with the repo). Two different
   `.casting/` dirs — `~/.casting/` (registry) vs `<repo>/.casting/` (state).
3. **No backwards compatibility, ever.** Drop the `--repo` alias and the
   `--project` path flag once the registry is in.

## CLI surface (new)

- `cast` (no args) — list projects from `~/.casting/projects.json`.
- `cast add <name> <repo-path>` — register a project (upsert by name).
- `cast remove <name>` — unregister.
- `cast run <project-name>` — resolve the repo via the registry, then boot.
- `cast init <dir> [--name=..]` — create the skeleton; **auto-register** the
   project (name = dir basename unless `--name`) so it's immediately runnable.
- `cast smoke <dir>` / `cast log --db <path>` — stay path-based (low-level).

## Files

| File | Change |
|---|---|
| `src/registry.rs` (new) | `Registry` (list of `{name, repo}`), load/save at `~/.casting/projects.json`, `register`/`remove`/`lookup` |
| `src/lib.rs` | `pub mod registry;` |
| `src/main.rs` | new dispatch: list / add / remove / run-by-name; init auto-registers; drop `--project`/`--repo` |
| `tests/registry.rs` (new) | registry CRUD + default + round-trip tests |

## Tasks (commit each)

1. registry.rs + tests (CRUD, empty default, upsert, round-trip)
2. CLI: list / add / remove / run-by-name + init auto-register; drop flags
3. docs (OWNERSHIP_BOUNDARY, HANDOFF, new-ish "registry" note) + gate + live verify

## Validation

- `make` full gate.
- Live: `cast add demo /path`, `cast` lists it, `cast run demo` boots; `cast init
  /tmp/x` auto-registers a `x` project.