# Seasoned Hand — single-image control plane (issue #6 / Phase 6.7).
#
# Multi-stage: build the Dioxus web bundle (Tailwind v4 standalone CLI, no Node)
# and the Rust control-plane binary, then ship a slim runtime that serves both the
# `/v1` + `/ws` API and the UI (via SH_UI_DIST). `rust:1-bookworm` (full, not slim)
# carries the C toolchain rusqlite's bundled SQLite needs.

# ---------- UI bundle (Dioxus wasm + compiled Tailwind) ----------
FROM rust:1-bookworm AS ui
WORKDIR /app
RUN rustup target add wasm32-unknown-unknown \
 && cargo install dioxus-cli --version 0.6.3 --locked
COPY . .
# Tailwind v4 standalone CLI (pinned version + sha256, matching justfile build-css —
# never exec an unverified downloaded binary). Purged CSS, no Node.
RUN curl -fsSL \
      https://github.com/tailwindlabs/tailwindcss/releases/download/v4.3.1/tailwindcss-linux-x64 \
      -o /tmp/tailwindcss \
 && echo "2526d063ba03b71f9a3ea7d5cee14f0aec147f117f222d5adc97b1d736d45999  /tmp/tailwindcss" \
      | sha256sum -c - \
 && chmod +x /tmp/tailwindcss \
 && mv /tmp/tailwindcss /usr/local/bin/tailwindcss \
 && mkdir -p crates/seasoned-hand-ui/assets \
 && tailwindcss -i crates/seasoned-hand-ui/input.css \
      -o crates/seasoned-hand-ui/assets/tailwind.css --minify
RUN cd crates/seasoned-hand-ui && dx build --platform web --release

# ---------- Control-plane binary ----------
FROM rust:1-bookworm AS backend
WORKDIR /app
COPY . .
RUN cargo build --release -p seasoned-hand-server

# ---------- Runtime ----------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=backend /app/target/release/seasoned-hand-server /usr/local/bin/seasoned-hand-server
COPY --from=ui /app/target/dx/seasoned-hand-ui/release/web/public /app/ui
# Defaults for the containerized deployment (override via compose/.env). The server
# binds loopback by default — in a container it must bind 0.0.0.0. SH_UI_DIST makes
# the control plane self-serve the bundle. DB + workspaces live under /app/data
# (mount a volume to persist). REDIS_URL points at the compose `redis` service.
ENV HOST=0.0.0.0 \
    PORT=3000 \
    SH_UI_DIST=/app/ui \
    DATABASE_URL=sqlite:/app/data/seasoned-hand.db \
    SANDBOX_WORKSPACE_HOST=/app/data/workspaces \
    REDIS_URL=redis://redis:6379 \
    SLOTS_CONFIG_PATH=/app/config/slots.yaml
RUN mkdir -p /app/data/workspaces
EXPOSE 3000
CMD ["seasoned-hand-server"]
