# syntax=docker/dockerfile:1
#
# Casting — build the single self-contained binary (with the embedded SPA) and
# run it in a container. Optional for users who prefer a container over running
# the binary directly.
#
# Multi-stage:
#   1. node   — build the React SPA into frontend/dist/   (rust-embed needs it)
#   2. rust   — cargo build --release (embeds frontend/dist/)
#   3. runtime — a tiny debian-slim with just the binary + a non-root user.
#
# Build:    docker build -t casting .
# Run (see docs/DEPLOYMENT.md / docs/HANDOFF.md for the full CLI):
#   docker run --rm -p 8080:8080 \
#     -v "$HOME/.casting:/home/casting/.casting" \
#     -v "/path/to/project:/home/casting/projects/demo" \
#     casting run my-project
#   docker run --rm casting --help        # explore the CLI
#
# Casting's state lives in the user's ~/.casting/ registry of projects (one
# directory per project, each holding its own state + port — never inside the
# repo). Mount that so a container run sees the same projects as a host run.

# ---- Stage 1: build the SPA -------------------------------------------------
FROM node:22-alpine AS frontend
WORKDIR /src
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# ---- Stage 2: build the Rust binary (embeds frontend/dist) ------------------
# Pin our stable toolchain. Debian slim keeps the final build small; we keep a
# git-less copy so the self-identity guard emits no stray source-root (the
# runtime binary we ship shouldn't claim a source repo).
FROM rust:1.97-slim-bookworm AS build
WORKDIR /app
COPY . .
# Overlay the freshly built SPA so rust-embed compiles the REAL UI.
COPY --from=frontend /src/dist /app/frontend/dist
RUN cargo build --release --locked

# ---- Stage 3: minimal runtime ----------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 casting
WORKDIR /home/casting
COPY --from=build /app/target/release/cast /usr/local/bin/cast
# The runtime user's home holds the ~/.casting/ registry.
USER casting
EXPOSE 8080
ENV CAST_ADDR=0.0.0.0:8080
ENTRYPOINT ["cast"]
CMD ["--help"]
