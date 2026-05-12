# Bifrost Setup

LLM gateway for Seasoned Hand.

## Quick Start

1. Copy `.env.example` to `.env` and fill in `ANTHROPIC_API_KEY` and `OPENAI_API_KEY`.
2. Start the gateway: `docker compose up -d bifrost`.
3. Run the smoke test: `./scripts/test-bifrost.sh`.

`local-fast` uses Ollama at `OLLAMA_BASE_URL` and is optional in Phase 0. If
Ollama is not reachable, the smoke test logs a skip and still exits 0.

## Configuration

Bifrost v1.5.0 reads `/app/data/config.json`; Docker Compose mounts the
checked-in `bifrost/config.json` there as read-only. `bifrost/config.yaml` is
the checked-in story intent template and should stay aligned with the JSON
config.

Model defaults are controlled by environment variables:

- `BIFROST_MODEL_PRIMARY`, default `claude-sonnet-4-6`
- `BIFROST_MODEL_FALLBACK`, default `gpt-4o`
- `BIFROST_MODEL_LOCAL_FAST`, default `llama3.2:3b`
