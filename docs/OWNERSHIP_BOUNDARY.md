# Casting — The Ownership Boundary (state vs. repos, and self-hosting)

Status: Architectural decision (D5)
Date: 2026-08-09
Author: Hermes, on behalf of owner (Ben)
Relates to: ADDENDUM §18–30 (Git/provenance), handoff §5 (Git slice)

This document settles a wrinkle that must be decided **before** the Git slice is
built, because it determines the workspace/path model the whole slice is built
on.

## 1. The problem: two (or more) repos with different authority

When Casting drives a Git workflow it must never confuse the repositories it
operates on. There are two clearly distinct repos in the normal case:

- **The Casting *source* repo** — the one that built the running `cast` binary
  (e.g. `/home/ben/casting`). It is product code, edited by humans/dev machines.
  **It is untouchable by agent actions.**
- **The *target* repo the user hands Casting** — the workspace Casting
  orchestrates, where agents create branches, commits, ChangeSets. This is the
  only repo agents may act on, and per ADDENDUM §18 it is treated as a
  first-class *external* system, never merged with Casting's own machinery.

Without a mechanical boundary, an agent (or a careless `cast run .`) can resolve
its target to the Casting source repo and start branching/committing in the
product tree.

## 2. The core principle / mental model

> **Casting operates on exactly one repo — the one it is explicitly handed at
> startup — via absolute paths derived from a single workspace root. By
> construction it never conducts on the repo that built it.**

The boundary must be **mechanical, not conventional**. Nobody may have to
*remember* to avoid the Casting repo; the binary enforces it.

## 3. Self-identity guard (the airtight refusal)

`build.rs` already runs on every compile. Extend it to emit the **git root of
the Casting source** (walk up from `CARGO_MANIFEST_DIR` until a `.git` is found)
plus the dev identity (`Cargo.toml` package `name = "casting"`) as embedded
constants, and the **HEAD sha**.

At startup, `cast run|init|smoke` resolves its target to a canonical absolute
path and **refuses to operate** if that path is:

- inside the embedded source root,
- an ancestor of the embedded source root, or
- the same git repo as the embedded root (walk up to top-level `.git`).

This kills the "agent writes into the Casting repo" class outright. It is fully
effective for the local/same-machine case that actually bites us; for a
distributed user it is a best-effort identity guard (the embedded root is the
builder's path), which is acceptable.

## 4. Path sandboxing & git scoping (agent-side enforcement)

Identity refusal is necessary but not sufficient — the repo may drift into the
hands of a *different* process later. Two runtime guardrails back it up:

- **One canonical root, absolute paths only.** Resolve the target to an
  absolute path at startup and make it the single `Workspace::root()`. Agents
  and every tool call receive **absolute paths derived from root**, never from
  ambient cwd, and each resolved path is validated to stay **under** root before
  execution (reject `..` traversal and any absolute path escaping root).
- **A single Git execution interface.** All Git commands — from agents, the
  PM, or any consumer — go **exclusively** through one `git` runner owned by
  `workspace.rs` that pins the repo (`-C <root>` plus `GIT_DIR`/`GIT_WORK_TREE`
  env set in the child) and never hands raw, unscoped `git` shells to agent
  code. So even a bare `git commit` with no args cannot reach the Casting repo
  (the classic "forgot to cd" accident). This interface is also where the §3
  ownership checks are enforced **for every call**, not just once at startup.

## 5. Preflight banner

`cast run` prints, before doing anything, the canonical workspace path, the
detected repo HEAD, and — if it detected the Casting source — a loud refusal.
The operator *sees* the target every time, so a foot-gun is obvious before
anything mutates.

## 6. State is COLLOCATED in a gitignored `.casting/` (one project dir)

The event store, cursors, agent state and `config.json` (which holds the owner
token) all live in **`<repo>/.casting/`** — *inside* the project repo, but
**self-ignored by git** so it never shows up as pending changes and is never a
committed or commit-able artifact.

- **One parameter, by design.** `cast run <dir>` — the project IS its
  git repo, and Casting derives `<dir>/.casting/` internally. No separate
  `--state-dir` to manage or mis-name.
- **Self-ignored, enforced by gitignore not discipline.** `cast init` / `cast
  run` create `<repo>/.casting/.gitignore` containing just `*`, so the entire
  directory is ignored without touching the user's root `.gitignore`. The repo
  stays a clean view of *the product's code only*.
- **Secrets never commit.** The owner token lives only in the gitignored
  `.casting/config.json`. A committed `casting.example.json` *template* (no
  token, like `.env.example`) documents the shape.
- **Migrating between machines** = clone the repo + `cast init` + copy the
  `.casting/` dir over (a file copy — databases can't be merged, so committing
  them would buy nothing).

Why this changed: the original `state-dir` was a heavy fix for the real (small)
problem — Casting's bookkeeping showing up as pending changes in the repo. A
gitignored dot-dir solves that directly and removes a second parameter.

Note on the end user: a normal Casting user never touches the Casting source, so
the §3 self-identity guard is mostly inert for them — their real protection is
the §6 self-ignored `.casting/` + the §4 git interface. The guard exists
primarily to protect our own dev/agents from the meta-confusion.

## 7. Self-hosting: building Casting with Casting

When we eventually dogfood (run Casting on the Casting source), the two repos of
§1 become the **same** repo: the target IS the source. This is a genuinely
different case and the default refusal in §3 would (correctly) block it. It must
be an **explicit escape hatch**, not an implicit allowance:

- **Explicit opt-in.** Driving the embedded source root requires
  `CAST_SELFHOST=1` (env) or `--selfhost`. Without it, the §3 refusal applies.
  With it, the guard demotes from refusal to a loud banner.
- **State is self-ignored under §6.** The collocated `.casting/` is gitignored,
  so even when self-hosting, Casting's internals never become commit-able
  product artifacts. (For the most isolated self-hosting setup, run against a
  `git worktree` copy with the tree the agents edit distinct from your dev
  checkout.)
- **Record which Casting built it.** Every agent-run / ChangeSet is tagged with
  the running binary's build commit (embedded HEAD sha). So "changes made by
  Casting vX" are distinguishable from human edits and from changes made by an
  older/newer Casting.
- **The staleness boundary is explicit.** The running binary is built from one
  fixed commit; as agents edit and commit the source, HEAD runs ahead of the
  running binary. The tool must (a) log the binary's build commit vs. current
  HEAD mismatch, and (b) treat *rebuilding Casting itself* as an explicit,
  orthogonal step — never something the running instance performs on itself
  mid-loop. This prevents the feedback trap of Casting editing the code that is
  running, which changes behavior while it runs.
- **No self-triggering.** Casting's own bookkeeping must never enter its
  meaningful-event feed. With `events.db` self-ignored under `.casting/`, the
  bookkeeping never shows in git at all; any residual bookkeeping commits are
  excluded by the observer explicitly.
- **Recommended self-hosting setup.** Build a fresh binary from a feature
  branch and run it against a **`git worktree`** copy of the source on its own
  branch (state collocates in that worktree's `.casting/`, self-ignored). This
  way the tree the agents edit is not the tree your dev environment is actively
  using, and the main checkout stays clean.

## 8. Enforcement obligations (tests)

- **No test may ever target the product repo.** Git-slice tests create throwaway
  repos under `tempdir` (existing `tempfile` pattern), never in `/home/ben/casting`.
- A test asserts state is **collocated** in `<repo>/.casting/` and that
  `ensure_self_ignored` writes `<repo>/.casting/.gitignore` = `*`, so the state
  dir never appears as a pending change.
- A test asserts the git runner pins the repo for every call (e.g. a bare call
  cannot resolve to the Casting repo) and that `Workspace::open` **refuses** the
  embedded source root / a repo whose identity is `casting`.
- A test asserts `--selfhost` demotes that refusal to a banner while keeping the
  self-ignored `.casting/` (self-hosting). Encode these rules so they cannot
  regress.

## 9. What changes in the codebase

- `build.rs`: emit `CASTING_SOURCE_ROOT`, package identity, and HEAD sha.
- `src/workspace.rs` (new): `Workspace::open(repo, selfhost)` — derives the
  collocated `.casting/` state dir, resolve+refuse (self-identity guard), path
  sandboxing, `ensure_self_ignored`, and the **sole git runner** through which
  all Git executes.
- `src/main.rs` / `src/web.rs`: preflight banner; `cast run <dir> [--db]` (+ `--selfhost`).
- `docs/HANDOFF.md`: register as D5 (done).
- Tests: identity-refusal + self-hosting-positive cases; collocated `.casting/`
  + self-gitignore; git-runner pinning; tempdir-only git tests.

## 10. Related but out of scope here

- The actual Git semantics (branches, ChangeSet, provenance) — ADDENDUM §18–30.
- Whether each project might later hold its state at a user-chosen path, or a
  `~/.casting/` convention — a later UX decision; for now `--project` collocates
  `.casting/` inside the repo, self-ignored.