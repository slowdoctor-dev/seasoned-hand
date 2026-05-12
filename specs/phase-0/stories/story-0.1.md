# Story 0.1 — Bifrost Docker Setup

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: none (this is the first story)
> **Phase**: 0
> **Type**: infrastructure

---

## Goal

Get Bifrost LLM gateway running in Docker with at least one cloud model and one local model accessible via OpenAI-compatible API.

## Acceptance criteria

- [ ] Bifrost container running and healthy
- [ ] `curl http://localhost:4000/v1/models` returns model list
- [ ] `curl http://localhost:4000/v1/chat/completions` with `model: agent-primary` returns valid response
- [ ] `curl http://localhost:4000/v1/chat/completions` with `model: local-fast` returns valid response (Ollama)
- [ ] Cost tracking enabled and visible in Bifrost dashboard
- [ ] Configuration in `bifrost/config.yaml` checked into repo
- [ ] Docker Compose service definition in `docker-compose.yml`
- [ ] Smoke test script `scripts/test-bifrost.sh` passes

## Non-goals

- Full 12-slot configuration (later story)
- Frontend integration (Story 0.20+)
- Production deployment (Phase 6)

---

## Implementation steps

### 1. Create directory structure
```
bifrost/
├── config.yaml
├── data/                  # gitignored
└── README.md
docker-compose.yml         # at repo root
scripts/
└── test-bifrost.sh
```

### 2. Bifrost config

```yaml
# bifrost/config.yaml
providers:
  anthropic:
    api_key: ${ANTHROPIC_API_KEY}
  openai:
    api_key: ${OPENAI_API_KEY}
  ollama:
    base_url: http://host.docker.internal:11434

models:
  - name: agent-primary
    provider: anthropic
    model: claude-sonnet-4-6
  
  - name: agent-fallback
    provider: openai
    model: gpt-4o
  
  - name: local-fast
    provider: ollama
    model: qwen2.5:32b

routing:
  fallbacks:
    agent-primary: [agent-fallback, local-fast]

observability:
  cost_tracking: true
  log_level: info
```

### 3. Docker Compose service

```yaml
# docker-compose.yml
services:
  bifrost:
    image: maximhq/bifrost:latest
    container_name: seasoned-hand-bifrost
    restart: unless-stopped
    ports:
      - "4000:8080"
    volumes:
      - ./bifrost/config.yaml:/app/config.yaml:ro
      - ./bifrost/data:/app/data
    environment:
      - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}
      - OPENAI_API_KEY=${OPENAI_API_KEY}
    extra_hosts:
      - "host.docker.internal:host-gateway"
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 10s
      timeout: 3s
      retries: 3
```

### 4. Smoke test

```bash
#!/bin/bash
# scripts/test-bifrost.sh
set -e

echo "Test 1: Health check"
curl -fsS http://localhost:4000/health || exit 1

echo "Test 2: List models"
curl -fsS http://localhost:4000/v1/models | jq '.data | length'

echo "Test 3: agent-primary completion"
curl -fsS http://localhost:4000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "agent-primary",
    "messages": [{"role": "user", "content": "Say hello in one word"}]
  }' | jq '.choices[0].message.content'

echo "Test 4: local-fast completion"
curl -fsS http://localhost:4000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "local-fast",
    "messages": [{"role": "user", "content": "Say hello in one word"}]
  }' | jq '.choices[0].message.content'

echo "All tests passed."
```

### 5. README

```markdown
# Bifrost Setup

LLM gateway for Seasoned Hand.

## Quick start

1. Copy `.env.example` to `.env` and fill in API keys
2. `docker compose up -d bifrost`
3. `./scripts/test-bifrost.sh`
```

---

## Verification

```bash
# All must pass:
docker compose up -d bifrost
sleep 5
./scripts/test-bifrost.sh

# Manual:
# - Open http://localhost:4000/admin (or whatever Bifrost dashboard URL)
# - Verify models listed
# - Verify a recent call's cost
```

---

## Files changed

- `bifrost/config.yaml` (new)
- `bifrost/README.md` (new)
- `docker-compose.yml` (new at root, will be extended later)
- `scripts/test-bifrost.sh` (new)
- `.env.example` (new, includes ANTHROPIC_API_KEY, OPENAI_API_KEY)
- `.gitignore` (add bifrost/data/)

---

## Spec references

- `/specs/01-architecture/ARCHITECTURE.md` §5 (Bifrost as LLM gateway)
- `/specs/01-architecture/ARCHITECTURE.md` §3 (model routing)

---

## Commit message

```
feat(phase-0): story 0.1 - Bifrost Docker setup

- Add Bifrost service to docker-compose
- Configure 3 initial model aliases (cloud + local)
- Add smoke test script
- Verified: all aliases respond, fallback chain works

refs: /specs/phase-0/stories/story-0.1.md
```

---

## Notes for next story (0.2)

Bifrost is now reachable at http://localhost:4000/v1.
Rust control plane will use this as OpenAI-compatible base URL.
Save the URL in env: `BIFROST_BASE_URL=http://localhost:4000/v1`.
