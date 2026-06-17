# Migration Guide

> 한국어 요약: 마이그레이션은 Refinery가 시작 시 한 번만 적용합니다.  
> 새 바이너리를 실행하면 남은 마이그레이션이 자동으로 올라갑니다.

Seasoned Hand uses [Refinery](https://crates.io/crates/refinery) for SQLite
migrations. That means:

- migrations are applied at startup;
- each migration is **once-only**;
- applied files are checksum-validated on future boots;
- upgrading the app is usually enough — the new binary applies pending
  migrations automatically.

## What to expect

On startup, the server opens the configured `DATABASE_URL` and runs any pending
migrations before it begins serving traffic. If a migration fails, startup
fails. There is no separate migration command in the normal path.

The canonical policy details live in [`migrations/README.md`](../migrations/README.md).
That file explains the append-only / checksum rule and the table-rebuild
pattern used by newer schema changes.

## The historical flip that matters

The one migration span worth remembering is the Phase 4 → Phase 5 tenant flip:
`V013` through `V020`.

That sequence introduced the multi-tenant org/user surfaces and flipped the
Phase 2–4 mutable tables from nullable `tenant_id` to `NOT NULL` with backfills
and rebuilds. It is the historical reason the repo now treats tenant isolation
as a first-class invariant instead of an afterthought.

Later hardening migrations build on that baseline:

- `V021` task parent FK + indexes
- `V023` session-search FTS tokenizer
- `V024` SOP tenant scope
- `V025` curator decision typing split
- `V026` audit log hash-chain + append-only triggers

## Upgrade checklist

1. Pull the new release or branch.
2. Build or run the server binary.
3. Watch startup logs for migration errors.
4. If the server reaches `starting`, the schema is current.

If you need a more detailed schema policy reference, use
[`migrations/README.md`](../migrations/README.md) as the source of truth.
