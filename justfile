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

# Run frontend (Next.js) in dev mode
dev-frontend:
    cd frontend && pnpm dev

# === Verification gates ===

# Run all verification gates (must pass before commit)
verify: lint typecheck test spec-check
    @echo "✓ All verification gates passed"

# Lint (Rust + TypeScript)
lint:
    cargo clippy --all-targets -- -D warnings
    cargo fmt --check
    cd frontend && pnpm lint

# Type check (TypeScript)
typecheck:
    cd frontend && pnpm typecheck

# Run all tests
test: test-backend test-frontend

test-backend:
    cargo test --workspace

test-frontend:
    cd frontend && pnpm test

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
    rm -rf bifrost/data target frontend/.next frontend/node_modules

# === Bifrost-specific ===

# Test Bifrost endpoint
test-bifrost:
    ./scripts/test-bifrost.sh
