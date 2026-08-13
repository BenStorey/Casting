# Collocated `.casting/` — one-project-dir model — Implementation Plan

> **For Hermes:** implement with TDD; commit; push. This is a real refactor —
> it reverses the old §6 "state-dir always separate" rule. Do it carefully.

## The new model (owner decision 2026-08-10)

A project = its git repo. Casting's state lives **inside the repo**, in a
**gitignored** `<repo>/.casting/` directory. ONE parameter to `cast run`
(`--project <dir>`); no separate `--state-dir`.

- **Committed:** only the product code the agents build (and an optional
  `casting.example.json` config *template* — no secrets).
- **Never committed:** `.casting/` (events.db, snapshots.db, cursors.db,
  config.json with owner token). Enforced by a **self-gitignore**
  `<repo>/.casting/.gitignore` containing `*` — the whole dir is ignored
  without touching the user's root `.gitignore`.
- Migrating to another machine = clone + `cast init` + copy `.casting/` over
  (a file copy, not a git merge — DBs can't be merged).
- Self-hosting still requires the `--selfhost` opt-in (guard stays).

## Changes by file

### `src/workspace.rs`
- `Workspace::open(repo, selfhost)` — drop the separate `state_dir` arg.
  `state_dir` becomes `repo.join(".casting")` (the *casting dir*).
- Add `casting_dir()` accessor (the `.casting/` path).
- Remove `ensure_distinct` (state is now deliberately collocated).
- Keep: self-identity guard, `ensure_repo`, `git_command`, `resolve_under`,
  `head`, `current_branch` — unchanged.
- Add `ensure_self_ignored()`: write `<repo>/.casting/.gitignore` = `*` if the
  dir doesn't already have one (idempotent).

### `src/main.rs`
- `RunArgs`: drop `state_dir`; `repo` → `project`. `parse_run` handles
  `--project <dir>` (keep `--repo` as an alias for back-compat).
- `do_run`: `Workspace::open(&run.project, run.selfhost)`; use
  `ws.casting_dir()` everywhere `ws.state_dir` was used (store paths, config,
  AppState::with_state_dir, snapshot path).
- `do_init`: `cast init <project-dir>` — create the repo's `.casting/` +
  self-gitignore + DBs, run the SetupPlan.
- `do_smoke` / `do_log`: update path derivation to the collocated dir.

### `src/setup.rs`
- `SetupPlan::apply` still takes a dir (the `.casting/` dir).
- Add the self-gitignore write on init so a fresh project is immediately
  git-ignored and `.casting.example.json` (the template) is written to the
  repo root.

### `tests/ownership_boundary.rs`
- Rewrite the "distinct & non-nested" tests → now assert the OPPOSITE: state
  IS collocated under `.casting/`, that `.casting/` is self-ignored, and that
  the self-identity refusal still fires without `--selfhost` (on the real
  source repo).

### Docs
- `OWNERSHIP_BOUNDARY.md` §6: rewrite from "always separate" → "collocated in
  a gitignored `.casting/`; forced separation only for self-hosting."
- `HANDOFF.md`: CLI section (`cast run --project`), Data-locations note.

## Back-compat / deploy
- The systemd `cast-backend` unit currently passes `--state-dir
  /home/ben/casting-workspace/state`. I will update it (or note the change)
  so the dev instance uses the new single-param form.

## Validation
- `make` full gate.
- Live: `cast run --project <fresh>` → verify `.casting/` created, gitignored,
  `git status` clean, first-run wizard works, auth from config.json.