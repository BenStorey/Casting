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

SHELL := /bin/bash
CARGO_HOME := $(HOME)/.cargo
export PATH := $(CARGO_HOME)/bin:$(PATH)

# The workspace `cast run` drives. Defaults to the current directory when
# running `make dev` or `make run` — `.casting/` state lives in the project root.
# Override: make run REPO_DIR=/path/to/project
REPO_DIR ?= .
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

# Run the whole workspace from ONE binary (API + embedded UI) on the single
# project dir (no registry — Casting is single-project).
run: build
	mkdir -p $(REPO_DIR)
	CAST_ADDR=$(CAST_ADDR) ./target/debug/cast run $(REPO_DIR)

# Live UI dev: cast run (API on :8080) + Vite HMR (on :5000, proxies /api).
# Both in one shell; Ctrl-C stops both. Kill any stale processes on the ports first.
dev: build
	mkdir -p $(REPO_DIR)
	@echo "→ freeing ports :8080 and :5000…"
	@fuser -k 8080/tcp 5000/tcp 2>/dev/null || true
	@echo "→ starting cast run (API :$(CAST_ADDR)) + Vite HMR (:5000); Ctrl-C to stop both"
	trap 'kill 0' INT TERM EXIT; \
	RUST_LOG=info CAST_ADDR=$(CAST_ADDR) ./target/debug/cast run $(REPO_DIR) & \
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

# Purge all Casting state from the project dir — equivalent to
# `rm -rf .casting`. Ask for confirmation.
purge:
	@test -d $(REPO_DIR)/.casting || { echo "no state to purge at $(REPO_DIR)"; exit 0; }; \
	read -p "Delete .casting/? [y/N] " ans; \
	[ "$$ans" = "y" ] || { echo "aborted"; exit 0; }; \
	rm -rf $(REPO_DIR)/.casting && echo "✓ purged — ready for cast run"
