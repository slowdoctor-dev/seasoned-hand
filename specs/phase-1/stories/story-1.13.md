# Story 1.13 — Checkpoint Manager — V005 + commit-on-advance + `checkpoint_label`

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 1.10 (VERIFYING / SUSPENDED transitions finalized
> — for state guard semantics that 1.13b consumes), 1.3 (sandbox is a
> git working tree)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.6 (Checkpoint
> Manager spec — *commit half*), §3.3 (checkpoints table), §4.3
> (`checkpoint_label` tool).

---

## Goal

Persist phase advances as git commits inside the sandbox workspace.
On every `Plan{op:"advance"}` event, run `git add -A && git commit
-m "phase N: <title>"`, store the resulting SHA in a new `checkpoints`
table, emit `Misc{kind:"checkpoint_create"}`. Ship the LLM-visible
`checkpoint_label(label)` tool that attaches a human label to the
**next** advance. Rollback (internal tool + admin endpoint + opt-in
Verifier-driven rollback) is story 1.13b.

## Acceptance criteria

- [ ] Migration `V005__checkpoints.sql` per architecture §3.3 (full
      column set + `idx_checkpoints_session(session_id, created_at)`).
      Idempotent — re-runs against a populated DB without error.
- [ ] `seasoned-hand-core::checkpoint::CheckpointManager` is a Tokio
      task spawned at server startup. Subscribes to the event stream
      (Phase 0 global subscribe by kind) for `Plan` events.
- [ ] On each `Plan{op:"advance"}` event:
      1. Read pending label (if any) from
         `CheckpointLabelBuffer::take(session_id)`.
      2. Run in sandbox via `/v1/shell`:
         - `git -C /workspace add -A`
         - `git -C /workspace commit -q --allow-empty -m "phase <id>: <title>"`
           — `--allow-empty` so phases that didn't write workspace
           files still produce a stable HEAD anchor.
         - `git -C /workspace rev-parse HEAD` → captures SHA.
      3. INSERT `checkpoints { id, session_id, plan_phase_id, git_sha,
         label?, triggered_by_event_id, created_at }`.
      4. Emit `Misc{kind:"checkpoint_create", data: {checkpoint_id,
         plan_phase_id, git_sha, label?}}`.
- [ ] Failure mode: any of the three shell commands returning non-zero
      → emit `Misc{kind:"checkpoint_create", data: {ok:false, reason,
      plan_phase_id}}`. Do **not** INSERT a row (rollback for that
      phase is intentionally unavailable per architecture §8).
- [ ] `checkpoint_label(label: String)` LLM-visible tool:
      - Validates `label.len() ≤ 80` (rejects with
        `label_too_long{max:80}`).
      - Calls `CheckpointLabelBuffer::set(session_id, label)`.
      - Returns `{ok:true, label, applies_to:"next_phase_advance"}`.
- [ ] `CheckpointLabelBuffer` is an `Arc<DashMap<String, String>>`-style
      in-memory map; `take(session_id)` clears as it reads (one-shot).
      Persisted across worker restart is **not** required — a label
      issued mid-task is lost on restart, which is acceptable since
      labels are user-decoration.
- [ ] `GET /v1/sessions/:id/checkpoints?cursor=&limit=` — paginated
      newest-first list. Default limit 50.
- [ ] Tests:
      - `migration_v005_idempotent`.
      - `plan_advance_creates_checkpoint_row_and_misc` — mock the
        sandbox shell to return canned SHAs; emit a synthetic Plan
        event; assert one new row + one Misc.
      - `checkpoint_label_attaches_then_clears` — set label, fire
        advance, second advance has no label.
      - `commit_failure_emits_create_with_ok_false_and_no_row` —
        sandbox shell mock returns exit 1; assert Misc payload
        `ok:false` and table SELECT returns 0 rows for the phase.
      - `http_checkpoints_list_route_returns_paginated_json`.
      - `checkpoint_label_rejects_long_label` — 100-char input → tool
        error.

## Non-goals

- `checkpoint_rollback` tool — story 1.13b.
- Admin rollback endpoint — story 1.13b.
- Verifier-driven opt-in rollback wiring — story 1.13b.
- LLM-facing rollback exposure (explicit anti-goal — handled in 1.13b
  via the story-1.5 tool-mask layer).
- Per-user rollback attribution (phase-1/DEBT.md #11 — Phase 5).
- Workspace TTL (phase-1/DEBT.md #8 — tied to Phase 0 DEBT #16).

## Implementation steps

### 1. Migration

```sql
-- migrations/V005__checkpoints.sql
CREATE TABLE checkpoints (
  id                      TEXT PRIMARY KEY,
  session_id              TEXT NOT NULL REFERENCES sessions(id),
  plan_phase_id           INTEGER NOT NULL,
  git_sha                 TEXT NOT NULL,
  label                   TEXT,
  triggered_by_event_id   INTEGER NOT NULL,
  rolled_back_at          INTEGER,
  rolled_back_by          TEXT,
  created_at              INTEGER NOT NULL
);
CREATE INDEX idx_checkpoints_session ON checkpoints(session_id, created_at);
```

The `rolled_back_at` / `rolled_back_by` columns ship now (used by
1.13b) — the migration is one of two changes Phase 1 makes to the DB
schema, and we'd rather pay the migration cost once.

### 2. Module

```
crates/seasoned-hand-core/src/checkpoint/
  mod.rs           — CheckpointManager spawn + run
  git_in_sandbox.rs— commit_phase(session, phase_id, title) -> sha
  persistence.rs   — insert + list_by_session
  label_buffer.rs  — DashMap<SessionId, String> + set/take
  routes.rs        — GET list handler
  tests.rs
crates/seasoned-hand-core/src/tools/
  checkpoint_label.rs  — new (LLM-visible)
```

### 3. Manager run loop

```rust
pub async fn run(state: AppState, shutdown: CancellationToken) {
    let mut rx = state.events.subscribe_global_kind("Plan").await;
    while !shutdown.is_cancelled() {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            Some(ev) = rx.recv() => {
                if ev.data.get("op").and_then(Value::as_str) != Some("advance") { continue; }
                let sid     = ev.session_id.clone();
                let title   = ev.data["phase_title"].as_str().unwrap_or("").into();
                let phase_id: u32 = ev.data["plan_phase_id"].as_u64().unwrap_or(0) as u32;
                let label   = state.checkpoint_labels.take(&sid);
                match commit_phase(&state, &sid, phase_id, &title).await {
                    Ok(sha) => {
                        let cp_id = persistence::insert(&state, &sid, phase_id,
                            &sha, label.as_deref(), ev.event_id).await.ok();
                        state.events.emit_misc(&sid, "checkpoint_create", json!({
                            "checkpoint_id": cp_id, "plan_phase_id": phase_id,
                            "git_sha": sha, "label": label,
                        })).await.ok();
                    }
                    Err(e) => {
                        state.events.emit_misc(&sid, "checkpoint_create", json!({
                            "ok": false, "reason": e.to_string(), "plan_phase_id": phase_id,
                        })).await.ok();
                    }
                }
            }
        }
    }
}
```

### 4. `git_in_sandbox::commit_phase`

```rust
pub async fn commit_phase(state: &AppState, session_id: &str, phase_id: u32, title: &str)
    -> Result<String, SandboxError>
{
    let title_esc = title.replace('"', "\\\"");
    state.sandbox.shell_exec(session_id, "git -C /workspace add -A").await?;
    state.sandbox.shell_exec(session_id,
        &format!("git -C /workspace commit -q --allow-empty -m \"phase {phase_id}: {title_esc}\"")
    ).await?;
    let out = state.sandbox.shell_exec(session_id, "git -C /workspace rev-parse HEAD").await?;
    Ok(out.stdout.trim().to_string())
}
```

`shell_exec` returns `{exit_code, stdout, stderr}` — any non-zero
`exit_code` is mapped to `SandboxError::Shell { exit_code, stderr }`
inside the helper.

### 5. `checkpoint_label` tool

```rust
async fn dispatch(ctx: &ToolContext, args: Value) -> ToolOutput {
    let label: String = parse_arg(&args, "label")?;
    if label.len() > 80 {
        return ToolOutput::err("label_too_long", json!({"max": 80, "got": label.len()}));
    }
    ctx.checkpoint_labels.set(&ctx.session_id, &label);
    ToolOutput::ok(json!({"label": label, "applies_to": "next_phase_advance"}))
}
```

### 6. `Misc.kind` docs

Append `checkpoint_create` to the documented set. (`checkpoint_rollback`
is documented in story 1.13b.)

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core checkpoint::manager::
cargo test -p seasoned-hand-core checkpoint::label_buffer::
cargo test -p seasoned-hand-core checkpoint::routes::
cargo test -p seasoned-hand-core tools::checkpoint_label::
./scripts/spec-check.sh
```

Live: run a 2-phase synthetic session; `git -C /workspace log --oneline`
in the sandbox shows 3 commits (init + 2 phase commits). The DB
`checkpoints` table has 2 rows.

---

## Files changed

- `migrations/V005__checkpoints.sql` (new)
- `crates/seasoned-hand-core/src/checkpoint/mod.rs` (new)
- `crates/seasoned-hand-core/src/checkpoint/git_in_sandbox.rs` (new)
- `crates/seasoned-hand-core/src/checkpoint/persistence.rs` (new)
- `crates/seasoned-hand-core/src/checkpoint/label_buffer.rs` (new)
- `crates/seasoned-hand-core/src/checkpoint/routes.rs` (new)
- `crates/seasoned-hand-core/src/checkpoint/tests.rs` (new)
- `crates/seasoned-hand-core/src/tools/checkpoint_label.rs` (new)
- `crates/seasoned-hand-core/src/tools/registry.rs` (modify — register
  `checkpoint_label`)
- `crates/seasoned-hand-server/src/main.rs` (modify — spawn manager,
  register list route)
- `crates/seasoned-hand-core/src/events/misc.rs` (modify — document
  `checkpoint_create`)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.6 (commit-on-advance + `--allow-empty`
  rationale), §3.3 (checkpoints table), §4.3 (`checkpoint_label` tool),
  §8 (commit-fail failure mode), §5.1 (git via sandbox shell, no `git2`).

---

## Commit message

```
feat(phase-1): story 1.13 - Checkpoint Manager + checkpoint_label

- V005 migration: checkpoints table per architecture §3.3 (full column
  set including rolled_back_at/by columns story 1.13b will use)
- checkpoint::CheckpointManager Tokio task subscribes to Plan
  events, runs git add -A && git commit -q --allow-empty -m
  "phase N: <title>" + git rev-parse HEAD inside the sandbox; INSERTs
  {checkpoint_id, plan_phase_id, git_sha, label?,
  triggered_by_event_id}; emits Misc checkpoint_create
- commit failure (non-zero shell exit) emits Misc checkpoint_create
  {ok:false, reason} and skips the INSERT — rollback for that phase
  becomes unavailable per architecture §8
- checkpoint_label(label) LLM tool: validates ≤80 chars; sets a
  pending label consumed by the NEXT advance and cleared
- GET /v1/sessions/:id/checkpoints list route (paginated, newest first)
- 6 unit + axum route + migration tests

refs: /specs/phase-1/stories/story-1.13.md
```

---

## Notes for next story (1.13b)

Phase advances now produce real git history + a `checkpoints` row per
phase. Story 1.13b adds the rollback half: internal-only
`checkpoint_rollback` tool (masked from LLM via the story-1.5
`DefaultMaskPolicy`), the admin HTTP endpoint with loopback + token
checks, and the opt-in Verifier-driven rollback (default off per
phase-1/DEBT.md #3). The `rolled_back_at` / `rolled_back_by` columns
in V005 are filled by 1.13b's UPDATE path.
