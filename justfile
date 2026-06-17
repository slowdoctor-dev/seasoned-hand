# Seasoned Hand — Task runner
# Install just: brew install just  OR  cargo install just

tailwind_version := "v4.3.1"
tailwind_bin := "target/tools/tailwindcss"
ui_css_output := "crates/seasoned-hand-ui/assets/tailwind.css"
ui_dist := "target/dx/seasoned-hand-ui/release/web/public"

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
dev-ui: build-css
    cd crates/seasoned-hand-ui && dx serve --platform web

# Build the Tailwind v4 stylesheet with the pinned standalone CLI (no Node).
build-css: _tailwind-cli
    mkdir -p crates/seasoned-hand-ui/assets
    {{tailwind_bin}} -i crates/seasoned-hand-ui/input.css -o {{ui_css_output}} --minify
    test -s {{ui_css_output}}

# Build the Dioxus UI to a static web bundle (output under target/dx/).
build-ui: build-css
    cd crates/seasoned-hand-ui && dx build --platform web --release
    test -d {{ui_dist}}

# Gate the Dioxus UI (no dx CLI needed). The UI crate is excluded from the root
# workspace, so the root cargo fmt/clippy/test gates do NOT cover it — this recipe
# is the only quality gate for the now-canonical UI, so it runs fmt + clippy +
# check, all on the wasm target.
check-ui:
    cargo fmt --manifest-path crates/seasoned-hand-ui/Cargo.toml -- --check
    cargo clippy --manifest-path crates/seasoned-hand-ui/Cargo.toml --target wasm32-unknown-unknown -- -D warnings
    cargo check --manifest-path crates/seasoned-hand-ui/Cargo.toml --target wasm32-unknown-unknown
    # Release check too: `dx build --release` turns off `debug_assertions`, which
    # disables rsx hot-reload and changes capture semantics (a value used in
    # `key: "{x}"` and also moved into an onclick closure compiles in debug but is
    # an E0382 in release). The debug check above cannot see that class of bug, so
    # the deploy image build was the only thing catching it (issue #6).
    cargo check --manifest-path crates/seasoned-hand-ui/Cargo.toml --target wasm32-unknown-unknown --release

_tailwind-cli:
    #!/usr/bin/env bash
    set -euo pipefail
    bin="{{tailwind_bin}}"
    if [[ -x "$bin" ]]; then
      exit 0
    fi
    os="$(uname -s)"
    arch="$(uname -m)"
    # Pin BOTH the version and a per-asset sha256 (issue #33 review): build-css
    # runs in PR CI, so an unverified downloaded executable would be arbitrary
    # code execution on a mutated/MITM'd release. Digests are for {{tailwind_version}};
    # bump them whenever tailwind_version changes (recompute with `sha256sum`).
    case "$os:$arch" in
      Linux:x86_64) asset="tailwindcss-linux-x64";  sha="2526d063ba03b71f9a3ea7d5cee14f0aec147f117f222d5adc97b1d736d45999" ;;
      Linux:aarch64|Linux:arm64) asset="tailwindcss-linux-arm64"; sha="3d662377a86d71c43b549dc06b90db4586b4acd412bf827a3268e951661e5adf" ;;
      Darwin:x86_64) asset="tailwindcss-macos-x64"; sha="e9e830ceb3e70b7e0775a3dd79eee8ec82c6b31270f08f2fa2857d0077045ac3" ;;
      Darwin:arm64|Darwin:aarch64) asset="tailwindcss-macos-arm64"; sha="a27c43626185953ee19bdace1939c7601e55da654e0b2fc4461e3e29957aa739" ;;
      *) echo "unsupported Tailwind standalone CLI platform: $os/$arch" >&2; exit 1 ;;
    esac
    mkdir -p "$(dirname "$bin")"
    url="https://github.com/tailwindlabs/tailwindcss/releases/download/{{tailwind_version}}/$asset"
    tmp="$(mktemp)"
    curl -fsSL "$url" -o "$tmp"
    # Verify the digest BEFORE making it executable / moving it into place.
    if command -v sha256sum >/dev/null 2>&1; then
      actual="$(sha256sum "$tmp" | awk '{print $1}')"
    else
      actual="$(shasum -a 256 "$tmp" | awk '{print $1}')"
    fi
    if [[ "$actual" != "$sha" ]]; then
      rm -f "$tmp"
      echo "Tailwind CLI sha256 mismatch for $asset" >&2
      echo "  expected $sha" >&2
      echo "  actual   $actual" >&2
      exit 1
    fi
    chmod +x "$tmp"
    mv "$tmp" "$bin"

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
