# Story 0.1 — Bifrost Docker Setup

> **Status**: in-progress
> **Version**: v1.1 (Codex pre-flight review issues closed; supersedes v1.0)
> **Estimated**: 2 hours
> **Dependencies**: none (this is the first story)
> **Phase**: 0
> **Type**: infrastructure
> **Reads first**: `/specs/phase-0/architecture.md` §4.4 + §5.2 + §9 + §12

---

## Goal

Get Bifrost LLM gateway running in Docker with three model aliases
(two cloud, one local-optional) accessible via OpenAI-compatible API.

## Acceptance criteria

**Hard (must pass):**

- [ ] Bifrost container running and reporting healthy (`docker compose ps`
      shows `healthy`, healthcheck endpoint resolved against the pinned tag)
- [ ] `curl -fsS http://localhost:4000/health` returns 200
- [ ] `curl -fsS http://localhost:4000/v1/models` returns a non-empty model list
      (JSON `data` array length ≥ 1)
- [ ] `curl -fsS http://localhost:4000/v1/chat/completions` with
      `model: agent-primary` returns a valid OpenAI-shaped response
- [ ] **Fallback proven**: a sub-test runs the same request with
      `ANTHROPIC_API_KEY=sk-deliberately-invalid` (inline env override
      ONLY for that one call, never written to `.env`) and asserts
      Bifrost served the response via `agent-fallback`. Mechanism:
      response metadata or Bifrost log line.
- [ ] `curl -fsS http://localhost:4000/cost` returns a JSON object
      (Bifrost-native cost tracking enabled — no local DB callback in
      this story; that lands in story 0.16)
- [ ] `bifrost/config.yaml` checked in, using `${VAR:-default}` templating
      for model IDs (see architecture.md §4.4 "Model IDs are
      env-overridable")
- [ ] Bifrost image **pinned** in `docker-compose.yml` (NOT `:latest`).
      Implementer picks the highest stable tag from
      `https://hub.docker.com/r/maximhq/bifrost/tags`, commits both
      the tag here and in `specs/phase-0/architecture.md` §5.2
- [ ] Bifrost config schema verified against the pinned tag's docs;
      key names and nesting adjusted from the example YAML below if
      Bifrost's actual schema differs
- [ ] `docker-compose.yml` service definition committed
- [ ] Smoke test script `scripts/test-bifrost.sh` exists and passes
- [ ] `scripts/spec-check.sh` passes
- [ ] `.env` is in `.gitignore` (verify)

**Conditional (only if Ollama is installed locally):**

- [ ] `curl http://localhost:4000/v1/chat/completions` with
      `model: local-fast` returns valid response.
- [ ] If `OLLAMA_BASE_URL` is unreachable, smoke test SKIPS this check
      (exit code 0, with `SKIP: local-fast (ollama unreachable)`
      logged to stdout). Do **not** fail the story on missing Ollama —
      Ollama is an optional Phase 0 prerequisite, documented in
      `docs/getting-started.md`.

## Explicitly NOT required (resolves Codex review)

- ❌ Bifrost admin dashboard URL / page open in browser. Cost visibility
   in Phase 0 = `GET /cost` JSON. Any dashboard shipped by the pinned
   Bifrost tag is a manual-inspection nice-to-have, not an acceptance
   gate.
- ❌ Local DB callback for cost. SQLite doesn't exist yet (story 0.3);
   the DB-side cost ingestion is story 0.16.
- ❌ 12-slot routing logic. This story creates **Bifrost-side aliases
   only** (`agent-primary`, `agent-fallback`, `local-fast`). The
   Rust-side slot resolver is story 0.12.
- ❌ Auth headers on `curl` calls. Phase 0 runs Bifrost unauthenticated,
   bound to 127.0.0.1. `BIFROST_MASTER_KEY` in `.env.example` is
   Phase-5 scaffolding and **must not** be sent in story 0.1 requests
   (sending it should also not break — but the test asserts the
   no-auth path).

## Non-goals

- Full 12-slot configuration (story 0.12)
- Frontend integration (story 0.20+)
- Production deployment (Phase 6)
- Multi-region, multi-machine Bifrost (never, per ADR-005)

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

**Intent template** — verify and adjust key names against the pinned
Bifrost tag's actual schema during implementation:

```yaml
# bifrost/config.yaml
providers:
  anthropic:
    api_key: ${ANTHROPIC_API_KEY}
  openai:
    api_key: ${OPENAI_API_KEY}
  ollama:
    base_url: ${OLLAMA_BASE_URL:-http://host.docker.internal:11434}

models:
  - name: agent-primary
    provider: anthropic
    model: ${BIFROST_MODEL_PRIMARY:-claude-sonnet-4-6}

  - name: agent-fallback
    provider: openai
    model: ${BIFROST_MODEL_FALLBACK:-gpt-4o}

  - name: local-fast
    provider: ollama
    model: ${BIFROST_MODEL_LOCAL_FAST:-llama3.2:3b}

routing:
  fallbacks:
    agent-primary: [agent-fallback, local-fast]

observability:
  cost_tracking: true
  log_level: info
```

Notes:
- Default `local-fast` model is `llama3.2:3b` (~2 GB, runs on a laptop).
  Earlier wording suggested `qwen2.5:32b`; that's impractical for a Phase 0
  smoke test and is dropped.
- All three model IDs are **env-overridable** so users can swap defaults
  without editing committed config (per architecture.md §4.4).

### 3. Docker Compose service

```yaml
# docker-compose.yml
services:
  bifrost:
    image: maximhq/bifrost:<PIN-EXACT-TAG-HERE>   # NOT `latest`. See AC.
    container_name: seasoned-hand-bifrost
    restart: unless-stopped
    ports:
      - "127.0.0.1:4000:8080"     # bind to localhost only (Phase 0)
    volumes:
      - ./bifrost/config.yaml:/app/config.yaml:ro
      - ./bifrost/data:/app/data
    environment:
      - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}
      - OPENAI_API_KEY=${OPENAI_API_KEY}
      # BIFROST_MASTER_KEY intentionally NOT passed in Phase 0 (architecture.md §9)
    extra_hosts:
      - "host.docker.internal:host-gateway"
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 10s
      timeout: 3s
      retries: 3
```

Verify the healthcheck path (`/health`) against the pinned Bifrost tag's
docs; some versions expose `/healthz` or `/v1/health` instead. Adjust
both the compose healthcheck and the smoke test if needed.

### 4. Smoke test

```bash
#!/bin/bash
# scripts/test-bifrost.sh
set -euo pipefail

BIFROST="${BIFROST_BASE_URL_HOST:-http://localhost:4000}"

echo "Test 1: Health check"
curl -fsS "$BIFROST/health" >/dev/null

echo "Test 2: List models (length >= 1)"
n=$(curl -fsS "$BIFROST/v1/models" | jq '.data | length')
[ "$n" -ge 1 ] || { echo "FAIL: empty model list"; exit 1; }

echo "Test 3: agent-primary completion"
curl -fsS "$BIFROST/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{"model":"agent-primary","messages":[{"role":"user","content":"Say hello in one word"}]}' \
  | jq -e '.choices[0].message.content' >/dev/null

echo "Test 4: local-fast completion (conditional on Ollama)"
if curl -fsS --max-time 2 "${OLLAMA_BASE_URL:-http://localhost:11434}/api/tags" >/dev/null 2>&1; then
  curl -fsS "$BIFROST/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -d '{"model":"local-fast","messages":[{"role":"user","content":"Say hello in one word"}]}' \
    | jq -e '.choices[0].message.content' >/dev/null
else
  echo "SKIP: local-fast (ollama unreachable at ${OLLAMA_BASE_URL:-http://localhost:11434})"
fi

echo "Test 5: Fallback chain (invalid Anthropic key -> agent-fallback)"
# Inline env override ONLY for this curl; never writes to .env.
# Bifrost reads its provider key from container env; restart with override:
# (Approach: temporarily push an invalid value through compose env-file shadowing
#  is awkward — instead, use Bifrost's own fallback API behavior:
#  send a request shaped to force a primary failure if the tag supports it,
#  OR restart bifrost with ANTHROPIC_API_KEY=sk-invalid for the duration of
#  this sub-test, then restore. Implementer chooses the cleanest path that
#  doesn't mutate the developer's real .env.)
ANTHROPIC_API_KEY=sk-deliberately-invalid \
docker compose up -d --no-deps --force-recreate bifrost >/dev/null
sleep 3
resp=$(curl -fsS "$BIFROST/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{"model":"agent-primary","messages":[{"role":"user","content":"Say hi"}]}')
echo "$resp" | jq -e '.choices[0].message.content' >/dev/null
# Restore by re-upping with real env (implementer should confirm the cleanest
# Bifrost-supported way to verify which model served the response):
docker compose up -d --no-deps --force-recreate bifrost >/dev/null

echo "Test 6: Cost endpoint reachable"
curl -fsS "$BIFROST/cost" | jq -e 'type == "object"' >/dev/null

echo "All tests passed."
```

Implementer note: if the pinned Bifrost tag exposes a cleaner "force
fallback" mechanism (e.g., a header, a model-mode flag), prefer that
over container-restart in Test 5.

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
