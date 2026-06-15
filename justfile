# Seasoned Hand — Task runner
# Install just: brew install just  OR  cargo install just

# Default: show available commands
default:
    @just --list

# === Stack lifecycle ===

# Start all services
up:
    docker compose up -d

# Stop all services
down:
    docker compose down

# Restart all services
restart: down up

# Tail logs (all services)
logs:
    docker compose logs -f

# === Development ===

# Run backend (Rust) in watch mode
dev-backend:
    cargo watch -x "run --bin seasoned-hand"

# Run the control plane WITHOUT Docker — SQLite-backed /v1 API on :3000 for dev
# and UI work (pair with `just dev-ui`). The SandboxClient connects to Docker
# lazily, and Redis degrades gracefully, so neither needs to be running. NOTE:
# executing a task (sandbox spawn) still requires Docker; this is for API/UI dev.
dev-server-nodocker:
    mkdir -p data/workspaces
    DATABASE_URL="sqlite:./data/seasoned-hand.db" \
    SANDBOX_WORKSPACE_HOST="./data/workspaces" \
    PORT="3000" \
    cargo run -p seasoned-hand-server

# Run the Dioxus UI (ADR-016) in dev mode. Requires the Dioxus CLI:
#   cargo install dioxus-cli   (provides `dx`)
dev-ui:
    cd crates/seasoned-hand-ui && dx serve --platform web

# Build the Dioxus UI to a static web bundle (output under target/dx/).
build-ui:
    cd crates/seasoned-hand-ui && dx build --platform web --release

# Gate the Dioxus UI (no dx CLI needed). The UI crate is excluded from the root
# workspace, so the root cargo fmt/clippy/test gates do NOT cover it — this recipe
# is the only quality gate for the now-canonical UI, so it runs fmt + clippy +
# check, all on the wasm target.
check-ui:
    cargo fmt --manifest-path crates/seasoned-hand-ui/Cargo.toml -- --check
    cargo clippy --manifest-path crates/seasoned-hand-ui/Cargo.toml --target wasm32-unknown-unknown -- -D warnings
    cargo check --manifest-path crates/seasoned-hand-ui/Cargo.toml --target wasm32-unknown-unknown

# === Verification gates ===

# Run all verification gates (must pass before commit)
verify: lint check-ui test spec-check
    @echo "✓ All verification gates passed"

# Lint (Rust). The frontend is now unified Rust (Dioxus, ADR-016).
lint:
    cargo clippy --all-targets -- -D warnings
    cargo fmt --check

# Run all tests
test: test-backend

test-backend:
    cargo test --workspace

# Verify code matches /specs
spec-check:
    ./scripts/spec-check.sh

# === Project status ===

# Show current phase, story, and blockers
status:
    @./scripts/status.sh

# === Story workflow ===

# Print prompt to start a new story execution session
story-prompt:
    @cat prompts/gsd-execute-story.md

# Print prompt to start phase planning (BMAD Analyst)
analyst-prompt:
    @cat prompts/bmad-analyst.md

# Print prompt to start architecture design (BMAD Architect)
architect-prompt:
    @cat prompts/bmad-architect.md

# Print prompt to start story breakdown (BMAD PM)
pm-prompt:
    @cat prompts/bmad-pm.md

# === Setup ===

# First-time setup
setup:
    cp .env.example .env
    @echo "✓ .env created — fill in API keys"
    @echo "Then run: just up"

# === Cleanup ===

# Remove all containers, volumes (DESTRUCTIVE)
clean:
    docker compose down -v
    rm -rf bifrost/data target

# === Bifrost-specific ===

# Test Bifrost endpoint
test-bifrost:
    ./scripts/test-bifrost.sh
