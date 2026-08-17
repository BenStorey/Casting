# Casting — dev/build/test/deploy in one command.
#
# The whole gate in a single step (default target):
#     make
#     # => fmt  -> clippy (-D warnings) -> test (all suites) -> build (embed real SPA)
#
# Named targets:
#     make dev      # run the full workspace (cast run + Vite HMR) for live UI work
#     make run      # run `cast run` only (API + embedded SPA on :8080)
#     make frontend # rebuild the real SPA (npm run build) into frontend/dist
#     make test     # cargo test only
#     make lint     # clippy --all-targets -- -D warnings
#     make fmt      # cargo fmt
#     make deploy-dev  # build + restart the dev-host services (dev.benstorey.com)
#     make restart     # restart cast-backend + cast-frontend systemd services
#
# Why "one step" matters: the SPA is embedded into the binary (rust-embed), so a
# frontend change must be npm-built BEFORE cargo build or you get the build.rs
# placeholder. This Makefile encodes that order so you never have to remember it.
#
# Rust lives in ~/.cargo (rustup standalone). Non-login shells don't have it on
# PATH, so we add it explicitly rather than relying on the environment.
#
# State layout: each project's state lives OUTSIDE the artifact repo, under
# ~/.casting/<slug>/ (honour $CASTING_HOME to relocate). `cast run` runs exactly
# one project per invocation — pass --project <slug>, or let it auto-select the
# sole project. To drive a specific project, set PROJECT to its slug.

SHELL := /bin/bash
CARGO_HOME := $(HOME)/.cargo
export PATH := $(CARGO_HOME)/bin:$(PATH)

# The project slug `cast run` drives. `cast run` auto-selects the sole project,
# or lists them and errors if more than one exists (then pass PROJECT=<slug>).
# Override: make run PROJECT=acme
PROJECT ?=
CAST_ADDR ?= 127.0.0.1:8080

.PHONY: all dev run frontend test lint fmt clean deploy-dev restart

# Default: the whole quality gate in one step.
all: fmt lint test build
	@echo "✓ all checks green"

# Order matters: frontend must be rebuilt BEFORE cargo build (embedded SPA).
build: frontend
	cargo build
	@echo "✓ built target/debug/cast (embedded real SPA)"

# Rebuild the real SPA into frontend/dist (tsc + vite build).
# Ensures node_modules exist first (idempotent — npm install is fast when cached).
frontend:
	cd frontend && npm install && npm run build

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt

# Run the whole workspace from ONE binary (API + embedded UI) for a given
# project. `cast run` auto-selects when exactly one project exists; otherwise
# pass PROJECT=<slug>. State lives under ~/.casting/<slug>/, never in the repo.
run: build
	@test -n "$(PROJECT)" || echo "tip: cast run auto-selects the sole project, or pass PROJECT=<slug>"
	CAST_ADDR=$(CAST_ADDR) ./target/debug/cast run $(if [ -n "$(PROJECT)" ]; then echo --project $(PROJECT); fi)

# Live UI dev: cast run (API on :8080) + Vite HMR (on :5000, proxies /api).
# Both in one shell; Ctrl-C stops both. Kill any stale processes on the ports first.
dev: build
	@echo "→ freeing ports :8080 and :5000…"
	@fuser -k 8080/tcp 5000/tcp 2>/dev/null || true
	@echo "→ starting cast run (API :$(CAST_ADDR)) + Vite HMR (:5000); Ctrl-C to stop both"
	trap 'kill 0' INT TERM EXIT; \
	CAST_ADDR=$(CAST_ADDR) ./target/debug/cast run $(if [ -n "$(PROJECT)" ]; then echo --project $(PROJECT); fi) & \
	cd frontend && npm run dev

# --- Dev-host deploy (dev.benstorey.com) ------------------------------------
# The long-lived systemd services (cast-backend + cast-frontend) serve
# dev.benstorey.com. They go stale when deps or the Rust binary change under
# them (a running Vite dev server does NOT survive a node_modules swap — it
# served a blank page after the v5->v8 upgrade until restarted). A deploy must
# restart them so the dev host can't silently go stale. One command:
#     make deploy-dev   # = build (re-embed SPA) + restart both services
.PHONY: deploy-dev restart
deploy-dev: build restart
	@echo "✓ dev host redeployed + services restarted — dev.benstorey.com live"

# Restart both dev-host services (requires sudo; Ben has NOPASSWD sudo).
#  - cast-backend:  serves the embedded SPA + /api on :8080
#  - cast-frontend: Vite HMR on :5173 (proxies /api -> :8080), fronted by Caddy
restart:
	sudo systemctl restart cast-backend cast-frontend
	@echo "✓ restarted cast-backend + cast-frontend"

clean:
	cargo clean

# Purge a project's state under ~/.casting/<slug> (full reset). Equivalent to
# `rm -rf ~/.casting/<slug>`. Pass the slug (or blank to target the sole project).
purge:
	@read -p "Project slug to purge? (leave blank for the sole project) " SLUG; \
	./target/debug/cast purge $${SLUG:-} --force && echo "✓ purged"
