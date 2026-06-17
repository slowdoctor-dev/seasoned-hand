# Configuration Reference

> 한국어 요약: 이 문서는 서버와 워커가 읽는 환경 변수를 정리합니다.  
> 아래 표는 실제 코드에서 확인한 값만 적었습니다. 운영 시나리오는 맨 아래를 보세요.

This page is the operational reference for the env vars the control plane and
background workers actually read. Defaults are taken from `main.rs` and the
core modules that own each component.

## Core server

- `DATABASE_URL`
  - Default: `sqlite:./data/seasoned-hand.db`
  - Used by: server startup / migrations
  - Notes: optional; if unset the server boots against the local SQLite file.

- `REDIS_URL`
  - Default: `redis://127.0.0.1:6379`
  - Used by: event bus, workers, pub/sub
  - Notes: optional; if Redis is unavailable some surfaces degrade, but the
    server still starts.

- `HOST`
  - Default: `127.0.0.1`
  - Used by: HTTP bind address
  - Notes: non-loopback binds are allowed, but the server warns loudly and you
    must place a trusted gateway in front.

- `PORT`
  - Default: `3000`
  - Used by: HTTP bind port

- `SANDBOX_WORKSPACE_HOST`
  - Default: `./data/workspaces`
  - Used by: sandbox workspace root on the host

- `AIO_SANDBOX_IMAGE`
  - Default: `ghcr.io/agent-infra/sandbox:1.0.0.152`
  - Used by: Docker sandbox client

- `SLOTS_CONFIG_PATH`
  - Default: `config/slots.yaml`
  - Used by: Bifrost slot router bootstrap
  - Notes: if missing or invalid, the server falls back to the built-in router.

- `SH_UI_DIST`
  - Default: unset
  - Used by: Dioxus bundle self-hosting
  - Notes: when set, must point at a built `dx build` directory containing
    `index.html`.

## Authentication and exposure

- `SEASONED_HAND_ADMIN_TOKEN`
  - Default: empty
  - Used by: admin rollback route
  - Notes: empty disables the route instead of allowing unauthenticated access.

- `SEASONED_HAND_INTAKE_TOKEN`
  - Default: empty
  - Used by: webhook intake route
  - Notes: empty disables the route.

- `SH_INSECURE_AUTH_HEADERS`
  - Default: off
  - Used by: legacy header-auth compatibility path
  - Notes: security-sensitive. Only enable on loopback / local dev / tests / CLI.

- `SEASONED_HAND_ROLLBACK_ON_VERIFIER_FAIL`
  - Default: `false`
  - Used by: verifier rollback behavior

- `SH_LEARNING_ENABLED`
  - Default: `true`
  - Used by: curator / learning extraction wiring

- `WEBHOOK_DELIVERY_ALLOWLIST`
  - Default: empty
  - Used by: webhook delivery SSRF bypass CIDRs
  - Notes: comma-separated CIDRs. Empty means default-deny only.

## LLM / routing / search

- `BIFROST_BASE_URL`
  - Default: `http://localhost:4000/v1`
  - Used by: slot router and direct OpenAI-compatible clients

- `BIFROST_MASTER_KEY`
  - Default: empty / unset
  - Used by: Bifrost client auth
  - Notes: optional.

- `BRAVE_API_KEY`
  - Default: empty / unset
  - Used by: web search client
  - Notes: optional. Without it, Brave search returns `MissingApiKey`.

## Email channel

The email channel is disabled unless `IMAP_HOST`, `IMAP_USERNAME`, and
`IMAP_PASSWORD` are all set.

- `IMAP_HOST`
  - Required to enable email intake
  - Default: empty

- `IMAP_USERNAME`
  - Required to enable email intake
  - Default: empty

- `IMAP_PASSWORD`
  - Required to enable email intake
  - Default: empty

- `IMAP_PORT`
  - Default: `993`

- `IMAP_POLL_INTERVAL_SECS`
  - Default: `30`
  - Notes: clamped to at least 1 second.

- `SMTP_HOST`
  - Default: `IMAP_HOST`

- `SMTP_USERNAME`
  - Default: `IMAP_USERNAME`

- `SMTP_PASSWORD`
  - Default: `IMAP_PASSWORD`

- `SMTP_PORT`
  - Default: `587`

- `EMAIL_FROM_ADDRESS`
  - Default: `SMTP_USERNAME`

- `EMAIL_SUBJECT_PREFIX`
  - Default: `[sh]`

- `INTAKE_EMAIL_ALLOWED_SENDERS`
  - Default: empty
  - Notes: empty means deny-all for inbound email.

## Notify / ntfy

- `NOTIFY_CONFIG_PATH`
  - Default: `config/notify.toml`
  - Notes: if missing or unreadable, notifications are disabled cleanly.

- `NTFY_TOPIC`
  - Default: unset
  - Used by: ntfy channel registration gate
  - Notes: the topic itself comes from `config/notify.toml`; this env var only
    says whether the operator wants ntfy enabled.

- `NTFY_HOST`
  - Default: `https://ntfy.sh`
  - Used by: ntfy transport

## Curator / learning / retention

- `SH_CURATOR_ENABLED`
  - Default: `false`

- `SH_CURATOR_INTERVAL_SECONDS`
  - Default: `300`

- `SH_CURATOR_BACKLOG_THRESHOLD`
  - Default: `10`

- `SH_CURATOR_MAX_CANDIDATES_PER_CYCLE`
  - Default: `50`

- `SH_CURATOR_EMBEDDING_BUDGET_MONTHLY_TOKENS`
  - Default: `50000`

- `SH_CURATOR_EMBEDDING_SOFT_CAP_PCT`
  - Default: `0.08`

- `SH_CURATOR_EMBEDDING_HARD_BREAKER_PCT`
  - Default: `0.12`

- `SH_CURATOR_REVIEW_SAMPLE_RATE`
  - Default: `0.30`
  - Notes: must stay within `0.0..=1.0`.

- `SH_EMBEDDING_MODEL`
  - Default: `text-embedding-3-small`

- `SH_CURATOR_AUTO_ARCHIVE_ENABLED`
  - Default: `false`

- `SH_CURATOR_ARCHIVE_RECOMMEND_MIN_CONFIDENCE`
  - Default: `0.40`

- `SH_CURATOR_ARCHIVE_APPLY_MIN_CONFIDENCE`
  - Default: `0.55`

- `SH_CURATOR_ORG_AGGREGATION`
  - Default: `false`

- `SH_CURATOR_PROJECT_ID`
  - Default: `default`

- `SH_CURATOR_L2_ENFORCE_KNOWLEDGE`
  - Default: `true`

- `SH_CURATOR_L2_ENFORCE_DATASOURCE`
  - Default: `true`

- `SH_CURATOR_ORGANIZATION_ID`
  - Default: `org-legacy-default`

- `SH_CURATOR_TENANT_ID`
  - Default: `legacy-default`

- `VERIFIER_PROMPT_PATH`
  - Default: `config/prompts/verifier.system.txt`
  - Notes: only read when the verifier slot is enabled.

- `NARRATOR_PROMPT_PATH`
  - Default: `config/prompts/narrator.system.txt`

- `SH_NOTIFY_ORGANIZATION_ID`
  - Default: `org-legacy-default`

- `SH_NOTIFY_TENANT_ID`
  - Default: `legacy-default`

- `SH_VERIFIER_ORGANIZATION_ID`
  - Default: `org-legacy-default`

- `SH_VERIFIER_TENANT_ID`
  - Default: `legacy-default`

- `SH_USER_COST_ORGANIZATION_ID`
  - Default: `org-legacy-default`

- `SH_USER_COST_TENANT_ID`
  - Default: `legacy-default`

- `SH_USER_COST_RECONCILE_ORGANIZATION_ID`
  - Default: `org-legacy-default`

- `SH_USER_COST_RECONCILE_TENANT_ID`
  - Default: `legacy-default`

- `SH_TTL_ORGANIZATION_ID`
  - Default: `org-legacy-default`

- `SH_TTL_TENANT_ID`
  - Default: `legacy-default`

- `SH_USER_COST_INTERVAL_SEC`
  - Default: `3600`

- `SH_USER_COST_RECONCILE_INTERVAL_SEC`
  - Default: `86400`

- `SH_CURATOR_RETENTION_INTERVAL_SEC`
  - Default: `86400`

- `SANDBOX_CLEANUP_INTERVAL_SEC`
  - Default: `3600`

- `SANDBOX_TTL_COMPLETED_DAYS`
  - Default: `30`

- `SANDBOX_TTL_FAILED_CANCELLED_DAYS`
  - Default: `7`

- `SANDBOX_TTL_DRAFT_DAYS`
  - Default: `1`

## Ambient / test-only inputs

These are read by helper code and tests, not by the normal deployment path.

- `HOME`
  - Used by: CLI deliverable fallback directory

- `HOSTNAME`
  - Used by: worker consumer IDs and verifier/notify host labels

- `RUN_DOCKER_TESTS`
  - Used by: Docker-dependent cache tests

- `REDIS_TEST_URL`
  - Used by: Redis test harnesses

- `BIFROST_TEST_MODEL`
  - Used by: LLM tests

## Scenario presets

### Local

Local means loopback control plane, local Docker sandbox, local Redis, and a
local Bifrost / Ollama setup if you want agent execution end-to-end.

```dotenv
HOST=127.0.0.1
PORT=3000
DATABASE_URL=sqlite:./data/seasoned-hand.db
REDIS_URL=redis://127.0.0.1:6379
AIO_SANDBOX_IMAGE=ghcr.io/agent-infra/sandbox:1.0.0.152
SANDBOX_WORKSPACE_HOST=./data/workspaces
BIFROST_BASE_URL=http://localhost:4000/v1
SH_INSECURE_AUTH_HEADERS=1
```

### Hybrid

Hybrid keeps the Docker sandbox local, but points the control plane at a remote
LLM gateway.

```dotenv
HOST=127.0.0.1
PORT=3000
DATABASE_URL=sqlite:./data/seasoned-hand.db
REDIS_URL=redis://127.0.0.1:6379
AIO_SANDBOX_IMAGE=ghcr.io/agent-infra/sandbox:1.0.0.152
SANDBOX_WORKSPACE_HOST=./data/workspaces
BIFROST_BASE_URL=https://bifrost.example.com/v1
BRAVE_API_KEY=...
SH_INSECURE_AUTH_HEADERS=0
```

### Cloud

Cloud here means the control plane and LLM gateway may be remote, but the cloud
sandbox provider is still deferred. Keep the Docker sandbox available until that
follow-up lands.

```dotenv
HOST=0.0.0.0
PORT=3000
DATABASE_URL=sqlite:/var/lib/seasoned-hand/seasoned-hand.db
REDIS_URL=redis://redis.example.com:6379
AIO_SANDBOX_IMAGE=ghcr.io/agent-infra/sandbox:1.0.0.152
SANDBOX_WORKSPACE_HOST=/var/lib/seasoned-hand/workspaces
BIFROST_BASE_URL=https://bifrost.example.com/v1
SH_UI_DIST=/opt/seasoned-hand/ui
SH_INSECURE_AUTH_HEADERS=0
```

In a real deployment, `HOST=0.0.0.0` must sit behind a trusted gateway that
handles caller identity before any `x-seasoned-hand-*` headers are trusted.
