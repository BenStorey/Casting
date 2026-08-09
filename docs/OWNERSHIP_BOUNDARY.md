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

## 6. State-dir is ALWAYS separate from the repo (mandatory, not optional)

The event store, cursors and agent state live in a `.casting/` directory. The
**state-dir and the artifact repo are always distinct paths, by construction** —
there is no collocated default to accidentally fall into.

- **`--state-dir <path>` is a required argument at startup.** It may live
  anywhere the operator chooses (sibling dir, a `~/.casting/workspaces/<name>`
  home, etc.), but it must never be inside the artifact repo, and the artifact
  repo must never be inside it.
- Example: `cast run --repo ~/proj --state-dir ~/.casting/workspaces/proj`.
- Because `.casting/` is never inside the repo, it is never a committed or
  commit-able artifact anywhere: Casting's internal state never pollutes a
  user's git history, and the repo remains a clean view of *the product's code
  only*.

This was originally motivated to make self-hosting safe; we make it **universal**
because it makes every case safer and eliminates a whole class of
"is this path the store or the source?" confusion — including the operator's own
finger-memory during development (the one place people were most likely to
collocate by habit).

Note on the end user: a normal Casting user never touches the Casting source, so
the §3 self-identity guard is mostly inert for them — their real protection is
this mandatory state-dir + the §4 git interface. The guard exists primarily to
protect our own dev/agents from the meta-confusion.

## 7. Self-hosting: building Casting with Casting

When we eventually dogfood (run Casting on the Casting source), the two repos of
§1 become the **same** repo: the target IS the source. This is a genuinely
different case and the default refusal in §3 would (correctly) block it. It must
be an **explicit escape hatch**, not an implicit allowance:

- **Explicit opt-in.** Driving the embedded source root requires
  `CAST_SELFHOST=1` (env) or `--selfhost`. Without it, the §3 refusal applies.
  With it, the guard demotes from refusal to a loud banner.
- **State separation is already enforced (§6).** `--state-dir` is mandatory and
  always distinct from the repo, so self-hosting gets it for free — `.casting/`
  can never land inside the monitored source tree, so Casting's internals can
  never become a commit-able product artifact.
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
  meaningful-event feed. With state separated out of the repo (above), the
  `.casting/` commits vanish, which mostly removes this risk; any residual
  bookkeeping commits are excluded by the observer explicitly.
- **Recommended self-hosting setup.** Build a fresh binary from a feature
  branch and run it against a **`git worktree`** copy of the source on its own
  branch, with `--state-dir` pointing outside the tree. This way the tree the
  agents edit is not the tree your dev environment is actively using, and the
  main checkout stays clean.

## 8. Enforcement obligations (tests)

- **No test may ever target the product repo.** Git-slice tests create throwaway
  repos under `tempdir` (existing `tempfile` pattern), never in `/home/ben/casting`.
- `Workspace::open` requires both a repo and a **distinct** state-dir; a test
  asserts it **errors** if the two are the same or nested.
- A test asserts the git runner pins the repo for every call (e.g. a bare call
  cannot resolve to the Casting repo) and that `Workspace::open` **refuses** the
  embedded source root / a repo whose identity is `casting`.
- A test asserts `--selfhost` demotes that refusal to a banner while keeping the
  mandatory separate state-dir. Encode these rules so they cannot regress.

## 9. What changes in the codebase

- `build.rs`: emit `CASTING_SOURCE_ROOT`, package identity, and HEAD sha.
- `src/workspace.rs` (new): `Workspace::open` — mandatory repo + distinct
  state-dir, resolve+refuse (self-identity guard), path sandboxing, and the
  **sole git runner** through which all Git executes.
- `src/main.rs` / `src/web.rs`: preflight banner; **required** `--state-dir`;
  `--selfhost`.
- `docs/HANDOFF.md`: register as D5 (done).
- Tests: identity-refusal + self-hosting-positive cases; state/repo distinctness;
  git-runner pinning; tempdir-only git tests.

## 10. Related but out of scope here

- The actual Git semantics (branches, ChangeSet, provenance) — ADDENDUM §18–30.
- Choosing a friendly *default* state-dir location for ergonomics (e.g. a
  `~/.casting/workspaces/<project-id>` home) — a later UX decision; it must
  stay outside the artifact repo regardless.