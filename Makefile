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

# The workspace `cast run` drives. Always outside the source tree (D5
# ownership-boundary guard refuses any repo nested under /home/ben/casting).
REPO_DIR := $(HOME)/casting-workspace/proj
PROJECT  := dev            # name in the ~/.casting/ registry
CAST_ADDR ?= 127.0.0.1:8080

.PHONY: all dev run frontend test lint fmt clean

# Default: the whole quality gate in one step.
all: fmt lint test build
	@echo "✓ all checks green"

# Order matters: frontend must be rebuilt BEFORE cargo build (embedded SPA).
build: frontend
	cargo build
	@echo "✓ built target/debug/cast (embedded real SPA)"

# Rebuild the real SPA into frontend/dist (tsc + vite build).
frontend:
	cd frontend && npm run build

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt

# Register the project in ~/.casting/ if missing, then run it by name.
# `cast add` is idempotent (upsert), so calling it every time is safe.
# Run the whole workspace from ONE binary (API + embedded UI).
run: build
	mkdir -p $(REPO_DIR)
	cast add $(PROJECT) $(REPO_DIR)
	CAST_ADDR=$(CAST_ADDR) ./target/debug/cast run $(PROJECT)

# Live UI dev: cast run (API on :8080) + Vite HMR (on :5173, proxies /api).
# Both in one shell; Ctrl-C stops both.
dev: build
	mkdir -p $(REPO_DIR)
	cast add $(PROJECT) $(REPO_DIR)
	@echo "→ starting cast run (API :$(CAST_ADDR)) + Vite HMR (:5173); Ctrl-C to stop both"
	trap 'kill 0' INT TERM EXIT; \
	CAST_ADDR=$(CAST_ADDR) ./target/debug/cast run $(PROJECT) & \
	cd frontend && npm run dev

clean:
	cargo clean
