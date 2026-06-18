#!/usr/bin/env bash
#
# test-docker-host.sh — run the Docker/Redis-dependent test tiers that CI skips
# by default (the `#[ignore]`'d sandbox + Redis suites). This seals the
# "cargo test --workspace green on a Docker host" release-readiness gate (#6).
#
# What it runs:
#   1. cargo test --workspace                         (non-ignored baseline)
#   2. cargo test --workspace -- --ignored pubsub::    (Redis pub/sub)
#   3. cargo test --workspace -- --ignored verifier::worker::  (Redis Streams)
#   4. RUN_DOCKER_TESTS=1 cargo test … --ignored sandbox::     (live Docker)
#
# What it does NOT run: the live LLM / SMTP / IMAP smokes (phase1_gaia,
# phase2_*, e2e_phase0, llm::live_bifrost_*) — those need provider API keys +
# Bifrost/SMTP and are a separate tier (see docs/configuration.md).
#
# Requirements: a running Docker daemon + cargo. Unless --no-sandbox is passed it
# pulls the pinned AIO sandbox image (~1GB, first run only). Redis is provided by
# a throwaway container (or an existing Redis already on :6379 is reused).
#
# Usage:
#   scripts/test-docker-host.sh               # full Docker + Redis tier
#   scripts/test-docker-host.sh --no-sandbox  # Redis tier only (skip ~1GB pull)
#   scripts/test-docker-host.sh --keep-redis  # leave the test Redis running
#
set -euo pipefail

AIO_IMAGE="ghcr.io/agent-infra/sandbox:1.0.0.152"
REDIS_CONTAINER="seasoned-hand-test-redis"
REDIS_PORT=6379
RUN_SANDBOX=1
KEEP_REDIS=0
STARTED_REDIS=0

for arg in "$@"; do
  case "$arg" in
    --no-sandbox) RUN_SANDBOX=0 ;;
    --keep-redis) KEEP_REDIS=1 ;;
    -h|--help) sed -n '2,33p' "$0"; exit 0 ;;
    *) printf 'unknown argument: %s (try --help)\n' "$arg" >&2; exit 2 ;;
  esac
done

# Run from the repo root regardless of where the script is invoked.
cd "$(dirname "$0")/.."

log()  { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m%s\033[0m\n' "$*" >&2; }
err()  { printf '\033[1;31mERROR: %s\033[0m\n' "$*" >&2; }

# ---- Preflight --------------------------------------------------------------
command -v cargo  >/dev/null 2>&1 || { err "cargo not found on PATH"; exit 1; }
command -v docker >/dev/null 2>&1 || { err "docker not found on PATH"; exit 1; }
docker info >/dev/null 2>&1 || {
  err "Docker daemon not reachable — start Docker and retry."
  exit 1
}

# True if something is already accepting TCP connections on the Redis port.
redis_up() { (exec 3<>"/dev/tcp/127.0.0.1/${REDIS_PORT}") 2>/dev/null; }

cleanup() {
  if [[ "${STARTED_REDIS}" == "1" && "${KEEP_REDIS}" == "0" ]]; then
    log "Removing test Redis container (${REDIS_CONTAINER})"
    docker rm -f "${REDIS_CONTAINER}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

# ---- Redis ------------------------------------------------------------------
if redis_up; then
  log "Reusing Redis already listening on 127.0.0.1:${REDIS_PORT}"
else
  log "Starting throwaway Redis (${REDIS_CONTAINER}) on 127.0.0.1:${REDIS_PORT}"
  docker rm -f "${REDIS_CONTAINER}" >/dev/null 2>&1 || true
  docker run -d --name "${REDIS_CONTAINER}" \
    -p "127.0.0.1:${REDIS_PORT}:6379" redis:7-alpine >/dev/null
  STARTED_REDIS=1
  for _ in $(seq 1 30); do redis_up && break; sleep 1; done
  redis_up || { err "Redis did not become reachable on :${REDIS_PORT}"; exit 1; }
fi
# pubsub:: and verifier::worker:: tests read REDIS_TEST_URL (then REDIS_URL).
export REDIS_TEST_URL="redis://127.0.0.1:${REDIS_PORT}"

# ---- AIO sandbox image ------------------------------------------------------
if [[ "${RUN_SANDBOX}" == "1" ]]; then
  log "Pulling pinned AIO sandbox image (${AIO_IMAGE}) — ~1GB, first run only"
  docker pull "${AIO_IMAGE}"
else
  warn "Skipping AIO image pull + Docker sandbox tests (--no-sandbox)."
fi

# ---- Tests ------------------------------------------------------------------
FAIL=0
run_step() {
  local title="$1"; shift
  log "${title}"
  if "$@"; then
    printf '\033[1;32m  ✓ %s\033[0m\n' "${title}"
  else
    err "${title} — FAILED"
    FAIL=1
  fi
}

run_step "1/4  Workspace tests (non-ignored baseline)" \
  cargo test --workspace
run_step "2/4  Ignored Redis pub/sub tests" \
  cargo test --workspace -- --ignored pubsub::
run_step "3/4  Ignored verifier-worker (Redis Streams) tests" \
  cargo test --workspace -- --ignored verifier::worker::
if [[ "${RUN_SANDBOX}" == "1" ]]; then
  run_step "4/4  Ignored live sandbox (Docker) tests" \
    env RUN_DOCKER_TESTS=1 cargo test --workspace -- --ignored sandbox::
else
  log "4/4  Skipped sandbox Docker tests (--no-sandbox)"
fi

# ---- Summary ----------------------------------------------------------------
if [[ "${FAIL}" == "0" ]]; then
  log "ALL DOCKER-HOST TESTS PASSED ✓"
else
  err "Some Docker-host tests FAILED — see the per-step output above."
fi
exit "${FAIL}"
