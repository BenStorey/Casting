# Casting CI — GitHub Actions — Implementation Plan

> **For Hermes:** implement; commit; push. Bring the CI green on the first run.

**Goal:** GitHub Actions CI for the **Casting repo itself** (a public OSS
project, so hosted Actions are free/unlimited). This is NOT the build location
for end-users' own projects (see the product note below). It mirrors the local
`make` gate on a clean, hosted machine so every push/PR to `main` is verified.

## Key design (owner input, 2026-08-12)

- **Where end-user builds run:** NOT GitHub Actions. A user running Casting on
  their own project builds locally, in the isolated worktrees we already built
  (each has its own `CARGO_TARGET_DIR` + API port → concurrent, on the user's
  machine, zero cost). The future **cloud build service is a deferred paid
  differentiator** (like the multi-project cloud tier), not GitHub Actions.
- **Casting's own CI:** GitHub Actions hosted runners — free for a public repo,
  separate machine, zero infra to run. This is the "builds on another machine"
  we get today.

## Workflow: `.github/workflows/ci.yml`

Mirrors `make` (fmt → clippy `-D warnings` → build → test), plus CI-only:

1. **Postgres service container** (postgres:16-alpine on :55432, user/db
   `casting`/`castpw`), so `tests/postgres_backend.rs` RUNS (it skips when PG
   is absent). Matches `deploy/docker-compose.postgres.yml` + the test default.
2. **Caching** (`~/.cargo`, `target/`, `node_modules` via npm ci cache) so a
   fresh runner each time stays fast.
3. **`cargo fmt --check`** (not `cargo fmt` — fails on drift instead of rewriting).
4. **Rust toolchain pinned to `1.97.1`** (matches local) via `dtolnay/rust-toolchain`.

Steps: checkout → setup node 22 → npm ci (frontend) → rust toolchain →
cache → fmt --check → clippy -D warnings → `npm run build` (frontend, MUST
precede cargo build for the embedded SPA) → `cargo build` → `cargo test` →
verify embedded SPA is the real build (not build.rs's placeholder).

`RUSTFLAGS=-D warnings` is set so the whole build (not just clippy) fails on
any warning — matching the project's "clippy 0, fmt clean" bar.

## Verification

Push to `origin/main`. CI runs automatically (public repo). Confirm the
`CI` workflow shows green: fmt, clippy, build, 217+ tests (with the Postgres
suite executing, not skipped).

## Product note to record (HANDOFF roadmap)

Local-first = builds run on the user's machine in worktrees (free). Cloud build
= deferred paid differentiator. GitHub Actions = Casting's own CI + optionally
a demonstration of local worktree builds.