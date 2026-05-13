# Story 2.18 — Verifier Worker real XREADGROUP loop (DEBT #15)

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: — (Phase 1 1.9b is base)
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-1/stories/story-1.9b.md`, `/specs/phase-1/DEBT.md` #15

---

## Goal

Replace the polling stub in `verifier::worker::Worker::run` with a real
`XREADGROUP` consumer + per-session FIFO + global concurrency cap.
This is the single biggest gap identified by the Phase 1 consistency
audit (S1.9b.1 + S1.9b.2): without this, the Verifier feature is a
library that nobody calls in production.

## Acceptance criteria

- [ ] `Worker::run` replaces the polling sleep loop with:
      ```
      XGROUP CREATE verify_request verifier-workers $ MKSTREAM (idempotent)
      loop {
          XREADGROUP GROUP verifier-workers <consumer-id> BLOCK 5000 COUNT 16 STREAMS verify_request >
          for each entry:
              parse VerifyRequest from JSON
              dispatch via per-session FIFO + global semaphore
              on dispatch end: XACK
              on parse failure: XACK + log + drop (no PEL retention for malformed
                                                   — they'd block forever)
      }
      ```
- [ ] Per-session FIFO: `DashMap<SessionId, Arc<Mutex<()>>>` ensures
      at most one in-flight `handle_request` per session_id.
- [ ] Global concurrency cap: `Arc<Semaphore>` with permits = config
      `verifier.max_concurrency` (default 2; was deleted in Phase 1
      simplicity M5; re-introduced here for the live consumer path).
- [ ] On `WorkerError` from `handle_request`: log + XACK (the error
      is already persisted as `verifier_verdict_error` Misc by the
      handler; PEL retention would block the queue).
- [ ] On crash between consume + ACK: Redis PEL retains the message;
      another consumer (or restart) picks it up. `handle_request`
      stays idempotent via the existing `triggered_at_event_id`
      dedup check on insert.
- [ ] Consumer id: `format!("worker-{hostname}-{pid}")` — distinct
      per process so multi-process scale-out doesn't collide.
- [ ] Configurable: `verifier.max_concurrency`, `verifier.consumer_block_ms`
      (default 5000), `verifier.read_count` (default 16) all
      env-loadable.
- [ ] Tests:
      - `worker_xreadgroup_consumes_one_entry` (live-Redis `#[ignore]`)
      - `worker_xreadgroup_per_session_fifo` (live-Redis `#[ignore]`):
        plant 3 messages for same session_id; assert serial handling
      - `worker_xreadgroup_global_semaphore_caps_concurrency`
        (live-Redis `#[ignore]`)
      - `worker_xack_on_handle_request_error` (live-Redis `#[ignore]`)
      - `worker_skips_malformed_message_with_xack` (live-Redis `#[ignore]`)
- [ ] All 5 tests are `#[ignore]`'d by default (require live Redis);
      `scripts/run-ignored-tests.sh` (new or existing — Phase 0
      0.27 pattern) runs them when `REDIS_URL` env points at a real
      Redis.

## Non-goals

- Replacing other Phase 1 polling stubs (e.g., CheckpointManager — its
  Plan{op:"advance"} broadcaster lands separately as part of the
  Plan-broadcaster wiring; DEBT #14 fix is a prereq for that, story
  2.19).
- Per-tenant queue isolation (Phase 5).
- Dead-letter queue for permanently failing handles (Phase 4 if
  observability needs it).

---

## Implementation steps

### 1. Re-introduce concurrency primitives

Phase 1 simplicity M5 dropped `tokio::sync::Semaphore` +
`DashMap<SessionId, Arc<Mutex<()>>>` from `verifier/worker.rs`
because nothing used them. This story re-introduces them with the
real consumer path that uses them.

### 2. XREADGROUP loop

Use existing `pubsub::RedisPool` for the connection. The Phase 0 0.5
pubsub module already has `xadd_json`; this story adds `xreadgroup`
+ `xack` helpers (or uses `redis::cmd("XREADGROUP")` directly with
the workspace-level Redis client).

### 3. Idempotency

`handle_request` already inserts into `verifications` table; the
insert's `triggered_at_event_id` is effectively a dedup key (the
same event_id can only have one row). If a retry inserts the same
verdict twice, the second insert is a `UNIQUE` violation surfaced
as `VerifierPersistenceError`. Worker logs + XACKs + moves on.

### 4. Consumer-group bootstrap

`XGROUP CREATE verify_request verifier-workers $ MKSTREAM` — runs
idempotently at worker boot. `MKSTREAM` creates the stream if it
doesn't exist; `$` starts from new entries. If the group already
exists, the `BUSYGROUP` error is swallowed.

### 5. Tests

Live-Redis tests use a unique stream name per test
(`format!("verify_request_test_{uuid}")`) to avoid cross-test pollution.

### 6. Strike-through Phase 1 DEBT #15

After this story commits, edit `specs/phase-1/DEBT.md` #15 to
strike-through with this commit's SHA.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core verifier::worker
REDIS_URL=redis://127.0.0.1:6379 cargo test -p seasoned-hand-core verifier::worker -- --ignored
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/verifier/worker.rs` (modify — replace
  polling stub with real XREADGROUP loop; re-introduce Semaphore +
  session FIFO map; new config knobs)
- `crates/seasoned-hand-core/src/verifier/worker/tests.rs` (modify —
  5 new `#[ignore]` tests)
- `crates/seasoned-hand-core/src/pubsub/mod.rs` (modify — XREADGROUP
  + XACK helpers if not present)
- `specs/phase-1/DEBT.md` (modify — strike-through #15)

---

## Spec references

- `/specs/phase-1/stories/story-1.9b.md` (original spec — sections
  on concurrency + consumer group)
- `/specs/phase-1/DEBT.md` #15 (the gap this story closes)

---

## Commit message

```
fix(phase-2): story 2.18 - Verifier Worker real XREADGROUP loop (DEBT #15 close)

The single biggest Phase 1 gap. Until this commit, Worker::run was
a 500ms polling shim — every trigger emission path successfully
XADD'd verify_request entries that no consumer ever read; verdicts
only flowed when test code called handle_request directly.

- Worker::run replaces polling sleep with XGROUP CREATE +
  XREADGROUP GROUP verifier-workers <consumer-id> BLOCK 5000
  COUNT 16 STREAMS verify_request >. Per-session FIFO via
  DashMap<SessionId, Arc<Mutex<()>>>; global semaphore for
  verifier.max_concurrency permits.
- handle_request error path: XACK + log + log Misc (no PEL
  retention for terminal errors).
- Malformed message path: XACK + log (no PEL retention for unparseable).
- Crash-between-consume-and-ACK: Redis PEL retains; another consumer
  picks up. handle_request idempotent via triggered_at_event_id dedup.
- Consumer id: worker-{hostname}-{pid}.
- 5 new tests under #[ignore] (require live Redis).

closes: Phase 1 DEBT #15

refs: /specs/phase-2/stories/story-2.18.md
```

---

## Notes for next story (2.19)

DEBT #15 closes. 2.19 closes DEBT #14 (SandboxGitShell shell-injection
fix) before the Plan{op:"advance"} broadcaster activates.
