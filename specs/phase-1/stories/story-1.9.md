# Story 1.9 — Verifier DB layer + V004 migration + `verifications` table + read routes

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 1.8 (verifier startup gate; `AppState::verifier_enabled`)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §3.1 (verifications
> table), §3.2 (sessions.state widening + transition table), §4.1 (read
> routes), `/specs/01-architecture/ARCHITECTURE.md` §6 L4.

---

## Goal

Ship the persistence layer for the Verifier without yet spawning the
runtime: V004 migration (new `verifications` table + `sessions.state`
CHECK widened to include `VERIFYING`), the verifier system-prompt
loader at server boot, persistence CRUD, and the two read-only HTTP
routes. The worker loop, Redis Streams consumer, fresh-context builder,
and watchdog are story 1.9b — splitting this story keeps each piece in
the 1-3h range and lets the DB layer land cleanly before any concurrent
work.

## Acceptance criteria

- [ ] Migration `V004__verifications.sql` is idempotent and atomic:
      - `CREATE TABLE verifications (...)` per architecture §3.1
        (full column set + indexes
        `idx_verifications_session(session_id, created_at)` and
        `idx_verifications_verdict(verdict)`).
      - `sessions.state` CHECK widened from
        `('IDLE','RUNNING','FINISHED','ERROR','SUSPENDED')` to add
        `'VERIFYING'`. Uses the SQLite "new-table-copy-drop-rename"
        pattern (the only path SQLite supports for CHECK changes).
- [ ] All **existing** indexes on the Phase 0 `sessions` table are
      re-created on the post-rename table. The migration test asserts
      this explicitly via `PRAGMA index_list('sessions')` comparison
      before-vs-after, not just by column count.
- [ ] `seasoned-hand-core::verifier::persistence` exposes
      `insert(req, verdict, model_id, cost_cents) -> VerificationId`,
      `get(id) -> Verification`, `list_by_session(session_id, cursor,
      limit) -> Vec<Verification>`.
- [ ] Server boot reads `/config/prompts/verifier.system.txt` into
      `AppState::verifier_system_prompt: Arc<String>` when
      `verifier_enabled` is true. If the file is missing, server fails
      to start with a clear error ("verifier prompt template missing
      at /config/prompts/verifier.system.txt"). The prompt content is
      the verbatim block from architecture.md §2.4.3.
- [ ] HTTP routes (read-only, no worker required):
      - `GET /v1/sessions/:id/verifications?cursor=&limit=` — newest-
        first paginated list (default limit 50).
      - `GET /v1/verifications/:id` — single row with
        `evidence_event_ids` and `suggested_plan_update` deserialised
        from their TEXT columns.
- [ ] Tests:
      - `migration_v004_idempotent_against_phase0_seed` — run V001-V003
        seed, then V004, then re-run V004; assert success + no
        duplicate columns.
      - `migration_v004_preserves_sessions_indexes` — assert all
        Phase 0 `sessions` indexes are present after V004 by name.
      - `persistence_insert_and_get_round_trip` — INSERT a synthetic
        row; SELECT returns the same shape.
      - `persistence_list_paginates_by_cursor` — INSERT 75 rows,
        list with `limit=50`, paginate via cursor for the rest.
      - `http_verifications_list_route_returns_paginated_json`.
      - `http_verification_by_id_route_returns_full_row_including_suggested_plan_update`.
      - `verifier_system_prompt_loaded_at_boot` — boot with
        `verifier_enabled=true` and a fixture prompt file; assert
        `AppState::verifier_system_prompt` contains the fixture text.
      - `verifier_system_prompt_missing_fails_boot` — boot with
        `verifier_enabled=true` and no file; assert startup error.

## Non-goals

- Worker loop, Redis Streams consumer, concurrency control, watchdog,
  fresh-context construction, verdict parsing — all story 1.9b.
- TaskComplete / Invalidation / CircuitBreaker triggers (stories 1.10,
  1.11, 1.12).
- VERIFYING state transitions in the agent runtime — story 1.10. The
  migration adds the value to the CHECK constraint; no code path
  *enters* the state yet.
- Frontend rendering of the new routes — story 1.18.

## Implementation steps

### 1. Migration

```sql
-- migrations/V004__verifications.sql

-- 1. New table.
CREATE TABLE verifications (
  id                        TEXT PRIMARY KEY,
  session_id                TEXT NOT NULL REFERENCES sessions(id),
  triggered_at_event_id     INTEGER NOT NULL,
  trigger_kind              TEXT NOT NULL CHECK(trigger_kind IN
                              ('TaskComplete','Invalidation','CircuitBreaker')),
  trigger_detail            TEXT NOT NULL,
  verdict                   TEXT NOT NULL CHECK(verdict IN ('pass','fail')),
  reason                    TEXT NOT NULL,
  evidence_event_ids        TEXT NOT NULL,
  suggested_plan_update     TEXT,
  model_id                  TEXT NOT NULL,
  cost_cents                INTEGER NOT NULL DEFAULT 0,
  created_at                INTEGER NOT NULL
);
CREATE INDEX idx_verifications_session ON verifications(session_id, created_at);
CREATE INDEX idx_verifications_verdict ON verifications(verdict);

-- 2. Widen sessions.state CHECK via temp table.
-- IMPORTANT: re-create every Phase 0 index on `sessions` (verbatim from
-- V001). The migration test asserts index parity.
CREATE TABLE sessions_new (
    -- columns identical to Phase 0 V001 except the state CHECK includes 'VERIFYING'
    id           TEXT PRIMARY KEY,
    state        TEXT NOT NULL CHECK(state IN
                  ('IDLE','RUNNING','FINISHED','ERROR','SUSPENDED','VERIFYING')),
    -- ... copy every other column verbatim from V001 ...
);
INSERT INTO sessions_new SELECT * FROM sessions;
DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;

-- Re-create indexes that existed on sessions in V001.
-- (List them here verbatim from V001. The migration test enforces no drift.)
-- e.g. CREATE INDEX idx_sessions_state ON sessions(state);
-- e.g. CREATE INDEX idx_sessions_created_at ON sessions(created_at);
```

The exact set of Phase 0 sessions indexes is read from
`migrations/V001__sessions.sql` and reproduced here. Implementer must
not invent new indexes here; the migration test
`migration_v004_preserves_sessions_indexes` enforces parity.

### 2. Module layout

```
crates/seasoned-hand-core/src/verifier/
  mod.rs             — re-exports + VerifyRequest/VerifyTrigger types
                       (the types are defined here so story 1.10/1.11/1.12
                       can construct them before the worker exists)
  persistence.rs     — insert / get / list_by_session
  routes.rs          — HTTP route handlers
  tests.rs
config/prompts/verifier.system.txt   — FAIL-biased prompt (verbatim §2.4.3)
```

### 3. Types

```rust
// verifier/mod.rs (types only — runtime in 1.9b)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyRequest {
    pub session_id: String,
    pub trigger: VerifyTrigger,
    pub triggered_at_event_id: u64,
    pub context_hint: VerifyContextHint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerifyTrigger {
    TaskComplete { final_message_call_id: String },
    Invalidation { reason: InvalidationReason },
    CircuitBreaker { kind: BreakerKind },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyContextHint;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InvalidationReason {
    FileMismatch { path: PathBuf, old_sha: String, new_sha: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum BreakerKind { Stuck, Cost, MaxSteps, ErrorRate }
```

`VerdictKind` (Pass | Fail) and `Verification` struct also defined here
so persistence + routes can use them.

### 4. Persistence

Straightforward rusqlite CRUD. `evidence_event_ids` stored as a JSON
array string (`"[1, 5, 17]"`); `suggested_plan_update` stored as
nullable JSON text.

### 5. Routes

```rust
pub async fn list_verifications(
    State(s): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let limit = q.limit.unwrap_or(50).min(200);
    let rows = s.verifications.list_by_session(&session_id, q.cursor, limit).await?;
    Ok(Json(ListResponse { rows, next_cursor: ... }))
}

pub async fn get_verification(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Verification>, ApiError> {
    let row = s.verifications.get(&id).await?;
    Ok(Json(row))
}
```

### 6. Prompt loader

```rust
// crates/seasoned-hand-server/src/state.rs
let verifier_system_prompt = if verifier_enabled {
    let path = "/config/prompts/verifier.system.txt";
    let txt = std::fs::read_to_string(path).map_err(|e| {
        anyhow::anyhow!("verifier prompt template missing at {path}: {e}")
    })?;
    Arc::new(txt)
} else {
    Arc::new(String::new())
};
```

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core verifier::persistence::
cargo test -p seasoned-hand-core verifier::routes::
cargo test -p seasoned-hand-core tests::migrations::v004
cargo test -p seasoned-hand-server verifier_system_prompt_loaded_at_boot
./scripts/spec-check.sh
```

---

## Files changed

- `migrations/V004__verifications.sql` (new)
- `crates/seasoned-hand-core/src/verifier/mod.rs` (new — types only)
- `crates/seasoned-hand-core/src/verifier/persistence.rs` (new)
- `crates/seasoned-hand-core/src/verifier/routes.rs` (new)
- `crates/seasoned-hand-core/src/verifier/tests.rs` (new)
- `crates/seasoned-hand-core/src/lib.rs` (modify — `pub mod verifier;`)
- `crates/seasoned-hand-server/src/state.rs` (modify —
  `verifier_system_prompt: Arc<String>`)
- `crates/seasoned-hand-server/src/routes/mod.rs` (modify — register
  list + get routes)
- `crates/seasoned-hand-core/tests/migrations.rs` (modify — 2 V004 tests)
- `config/prompts/verifier.system.txt` (new — verbatim §2.4.3)

---

## Spec references

- `/specs/phase-1/architecture.md` §3.1 (verifications table), §3.2
  (state widening), §4.1 (read routes), §2.4.3 (system prompt text).

---

## Commit message

```
feat(phase-1): story 1.9 - verifier DB layer + V004 migration + read routes

- V004 migration: verifications table (architecture §3.1) +
  sessions.state CHECK widened to include 'VERIFYING' via SQLite
  temp-table pattern; existing Phase 0 sessions indexes verbatim
  re-created (migration_v004_preserves_sessions_indexes asserts parity)
- verifier::persistence: insert/get/list_by_session CRUD over the new
  table; JSON-text columns for evidence_event_ids and
  suggested_plan_update
- verifier::routes: GET /v1/sessions/:id/verifications (paginated,
  newest first) + GET /v1/verifications/:id
- verifier system prompt loaded from /config/prompts/verifier.system.txt
  at boot when verifier_enabled=true; missing file fails startup
- VerifyRequest / VerifyTrigger / BreakerKind / Verification types
  defined here so stories 1.9b/1.10/1.11/1.12 can construct them
- 8 unit + axum route + migration tests

refs: /specs/phase-1/stories/story-1.9.md
```

---

## Notes for next story (1.9b)

DB layer and read routes are live; no events flow yet (worker not
spawned). Story 1.9b adds the runtime: Tokio task + Redis Streams
consumer + fresh-context builder + verdict parser + per-session FIFO +
global concurrency cap + watchdog. After 1.9b, a hand-crafted
`XADD verify_request *` produces a row in `verifications` and a Misc
`verifier_verdict` event.
