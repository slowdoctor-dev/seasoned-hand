#!/usr/bin/env bash
set -euo pipefail

BIFROST="${BIFROST_BASE_URL_HOST:-http://localhost:4000}"
OLLAMA="${OLLAMA_BASE_URL:-http://localhost:11434}"

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "FAIL: missing required command: $1" >&2
    exit 1
  fi
}

json_object_or_array() {
  jq -e 'type == "object" or type == "array"' >/dev/null
}

chat() {
  local model="$1"
  curl -fsS "$BIFROST/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -d "{\"model\":\"$model\",\"messages\":[{\"role\":\"user\",\"content\":\"Say hello in one word\"}]}"
}

need curl
need jq
need docker

if [ -f .env ]; then
  set -a
  # shellcheck disable=SC1091
  . ./.env
  set +a
fi

if [ -z "${ANTHROPIC_API_KEY:-}" ] || [ -z "${OPENAI_API_KEY:-}" ]; then
  echo "FAIL: ANTHROPIC_API_KEY and OPENAI_API_KEY must be set in .env for Story 0.1 cloud smoke tests" >&2
  exit 1
fi

echo "Test 1: Compose health"
docker compose ps bifrost | grep -E 'healthy|Up' >/dev/null

echo "Test 2: Health endpoint"
curl -fsS "$BIFROST/health" >/dev/null

echo "Test 3: List models (length >= 1)"
n="$(curl -fsS "$BIFROST/v1/models" | jq '.data | length')"
[ "$n" -ge 1 ] || { echo "FAIL: empty model list"; exit 1; }

echo "Test 4: agent-primary completion"
chat "agent-primary" | jq -e '.choices[0].message.content' >/dev/null

echo "Test 5: local-fast completion (conditional on Ollama)"
if curl -fsS --max-time 2 "$OLLAMA/api/tags" >/dev/null 2>&1; then
  chat "local-fast" | jq -e '.choices[0].message.content' >/dev/null
else
  echo "SKIP: local-fast (ollama unreachable at $OLLAMA)"
fi

echo "Test 6: Fallback chain (invalid Anthropic key -> agent-fallback)"
ANTHROPIC_API_KEY=sk-deliberately-invalid \
  docker compose up -d --no-deps --force-recreate bifrost >/dev/null
sleep 3
chat "agent-primary" | jq -e '.choices[0].message.content' >/dev/null
docker compose up -d --no-deps --force-recreate bifrost >/dev/null

echo "Test 7: Cost/log endpoint reachable"
if curl -fsS "$BIFROST/cost" 2>/dev/null | json_object_or_array; then
  :
else
  curl -fsS "$BIFROST/api/logs?limit=1&offset=0" | jq -e 'type == "object" and has("stats")' >/dev/null
fi

echo "All tests passed."
