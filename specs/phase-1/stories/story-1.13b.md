# Story 1.13b — Checkpoint rollback (internal tool + admin endpoint + opt-in Verifier path)

> **Status**: done
> **Estimated**: 2.5 hours
> **Dependencies**: 1.13 (checkpoint create path + table), 1.5
> (tool-mask layer — `checkpoint_rollback` is registered but never
> LLM-visible)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.6 (rollback
> half — note the `git revert --no-commit` choice and rationale),
> §4.1 (admin endpoint), §9 (admin auth posture), `/specs/phase-1/DEBT.md`
> #3 (opt-in default).

---

## Goal

Add the rollback side of checkpoints: an internal-only
`checkpoint_rollback` tool (registered in the catalog but
masked from every LLM-facing `AgentMode` by the story-1.5
`DefaultMaskPolicy`), an admin HTTP endpoint with loopback + token
checks, and a config-gated path for Verifier verdicts to invoke
rollback (defaults **off**). The LLM never sees git.

## Acceptance criteria

- [ ] `seasoned-hand-core::tools::checkpoint_rollback::CheckpointRollbackTool`
      registered in the tool catalog. The story-1.5 `DefaultMaskPolicy`
      already returns `is_available("checkpoint_rollback", _) == false`
      for every LLM-facing mode — verified by a regression test in
      *this* story.
- [ ] Tool body, given `{checkpoint_id, reason}`:
      1. Read the row via `checkpoint::persistence::get(checkpoint_id)`.
         404 if missing.
      2. Run `git -C /workspace revert --no-commit <git_sha>` inside
         the sandbox. **Rationale (architecture §2.6)**: revert
         preserves history via a forward inverse-patch commit; no
         `git reset --hard` because (a) we don't rewrite history,
         (b) the agent receives a follow-up system message
         describing the revert so its mental model stays consistent.
      3. UPDATE `checkpoints` row with
         `rolled_back_at = now_micros`, `rolled_back_by = "<actor>"`
         (string carried in `args.rolled_back_by`, default
         `"admin:cli"`).
      4. Emit `Misc{kind:"checkpoint_rollback", data: {checkpoint_id,
         git_sha, reason, rolled_back_by}}`.
      5. Inject a system message into the agent's context for the
         next iteration: `"A prior phase commit was rolled back
         (checkpoint <id>, reason: <reason>). Workspace state has
         been restored via inverse-patch commit."` — implemented as
         a Misc event the runtime's `build_messages` already surfaces,
         not as a tool-output side-effect.
      6. Return `ToolOutput::ok({checkpoint_id, rolled_back_at})`.
- [ ] HTTP route `POST /v1/sessions/:id/checkpoints/:checkpoint_id/rollback`:
      - Body: `{ reason: string }` (reason required, ≤ 200 chars).
      - Header: `X-Seasoned-Hand-Admin-Token: <env-supplied>`.
      - **Loopback guard**: reject `403 forbidden_non_loopback`
        if `connect.remote_addr().is_loopback() == false`.
      - **Token guard**: reject `401 unauthorized_token` if missing
        or wrong.
      - **State guard**: reject `409 wrong_state` when
        `sessions.state` ∈ `{RUNNING, VERIFYING}`. Allowed states
        for rollback: `{SUSPENDED, ERROR, FINISHED}`. (Architecture
        §4.1 says "refuses to rollback while session is RUNNING";
        we extend with VERIFYING for consistency — the verdict
        machinery is still in flight.)
      - **Sandbox-paused guard**: reject `409 sandbox_paused` if
        `bollard::inspect` shows the container paused; client must
        call `task_resume` (story 1.17) first.
      - On success: 202 Accepted, body
        `{checkpoint_id, rolled_back_at}`.
- [ ] Verifier-driven opt-in path:
      - Config `checkpoint.rollback_on_verifier_fail: bool` (default
        **false**, per phase-1/DEBT.md #3).
      - When true *and* the Gate (story 1.10) receives a
        `verifier_verdict{verdict:"fail"}` carrying
        `rollback_required: true` (a future-compat field; Phase 1
        verdict schema permits it but the default verifier prompt
        never emits it — config controls whether it's honored),
        the Gate calls `dispatcher.dispatch_internal("checkpoint_rollback",
        {checkpoint_id: <latest_for_session>, reason: "verifier_fail",
        rolled_back_by: "verifier"})`.
      - When false (Phase 1 default): the field is logged but
        ignored; rollback only via admin endpoint.
- [ ] Admin endpoint env var: `SEASONED_HAND_ADMIN_TOKEN` loaded at
      boot into `AppState::admin_token`. If unset, the route returns
      `503 admin_token_not_configured` rather than allowing
      unauthenticated access (PRINCIPLE #10 — fail visibly).
- [ ] Tests:
      - `mask_blocks_checkpoint_rollback_from_worker_mode` — regression
        of story 1.5.
      - `rollback_tool_runs_git_revert_and_marks_row` — sandbox shell
        mock; assert revert command + row UPDATE + Misc event.
      - `rollback_tool_emits_system_message_for_agent` — Misc event
        present that the runtime's `build_messages` filter surfaces.
      - `admin_rollback_happy_path` — axum client + valid token + state
        SUSPENDED → 202 + row updated.
      - `admin_rollback_refuses_while_running` → 409 + `wrong_state`.
      - `admin_rollback_refuses_while_verifying` → 409.
      - `admin_rollback_refuses_without_token` → 401.
      - `admin_rollback_refuses_non_loopback_remote` — synthesise a
        non-loopback `RemoteAddr` → 403.
      - `admin_rollback_refuses_when_sandbox_paused` → 409 +
        `sandbox_paused`.
      - `admin_rollback_503_when_admin_token_unset` — boot with the env
        var missing; assert 503.
      - `verifier_driven_rollback_disabled_by_default` — `fail` verdict
        with `rollback_required:true` does NOT invoke the tool when
        config is false; just logs.
      - `verifier_driven_rollback_invokes_when_enabled` — same, but
        config true → tool dispatched.

## Non-goals

- LLM exposure (forever; preserved by the story-1.5 mask layer).
- `git reset --hard` semantics. Architecture §2.6 explicitly chose
  `git revert --no-commit` to preserve history and inform the agent;
  changing this requires an ADR.
- Multi-checkpoint rollback in one call.
- Cancelling an in-flight rollback (Phase 5 if it ever matters).
- Frontend UI for triggering rollback — Phase 1 ships CLI/`curl` only.

## Implementation steps

### 1. Tool registration

```rust
// crates/seasoned-hand-core/src/tools/registry.rs
register(Box::new(CheckpointRollbackTool::default()));
```

Verify the story-1.5 `DefaultMaskPolicy` already returns
`is_available("checkpoint_rollback", _) == false` for `Worker`,
`Verifier`, and any other LLM-facing mode. Add a regression test
here even though it duplicates 1.5's coverage — it's cheap and the
invariant matters.

### 2. Tool body

```rust
async fn dispatch(ctx: &ToolContext, args: Value) -> ToolOutput {
    let checkpoint_id: String = parse_arg(&args, "checkpoint_id")?;
    let reason: String = parse_arg(&args, "reason")?;
    let rolled_back_by: String = args.get("rolled_back_by")
        .and_then(Value::as_str).unwrap_or("admin:cli").into();
    let row = match ctx.checkpoints.get(&checkpoint_id).await? {
        Some(r) => r,
        None => return ToolOutput::err("checkpoint_not_found", json!({"id": checkpoint_id})),
    };
    let cmd = format!("git -C /workspace revert --no-commit {}", row.git_sha);
    ctx.sandbox.shell_exec(&ctx.session_id, &cmd).await?;
    let rolled_back_at = now_micros();
    ctx.checkpoints.mark_rolled_back(&checkpoint_id, rolled_back_at, &rolled_back_by, &reason).await?;
    ctx.events.emit_misc(&ctx.session_id, "checkpoint_rollback", json!({
        "checkpoint_id": checkpoint_id, "git_sha": row.git_sha,
        "reason": reason, "rolled_back_by": rolled_back_by,
    })).await?;
    // No need to emit a separate "agent system message" event — the
    // Misc event is already surfaced by build_messages (Phase 0 §3
    // sticky context renders Misc as system-style entries).
    ToolOutput::ok(json!({"checkpoint_id": checkpoint_id, "rolled_back_at": rolled_back_at}))
}
```

### 3. Admin route

```rust
pub async fn post_rollback(
    State(s): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((session_id, checkpoint_id)): Path<(String, String)>,
    Json(body): Json<RollbackBody>,
) -> Result<(StatusCode, Json<RollbackResponse>), ApiError> {
    if s.admin_token.is_empty() { return Err(ApiError::ServiceUnavailable("admin_token_not_configured")); }
    if !addr.ip().is_loopback() { return Err(ApiError::Forbidden("forbidden_non_loopback")); }
    let token = headers.get("X-Seasoned-Hand-Admin-Token").and_then(|h| h.to_str().ok());
    if token != Some(s.admin_token.as_str()) { return Err(ApiError::Unauthorized("unauthorized_token")); }
    if body.reason.len() > 200 { return Err(ApiError::BadRequest("reason_too_long")); }

    let st = s.sessions.state(&session_id).await?;
    if matches!(st.as_deref(), Some("RUNNING" | "VERIFYING")) {
        return Err(ApiError::Conflict("wrong_state"));
    }
    let paused = s.sandbox.is_paused(&session_id).await?;
    if paused { return Err(ApiError::Conflict("sandbox_paused")); }

    let ctx = ToolContext::for_internal(&s, &session_id);
    let out = s.dispatcher.dispatch_internal(&ctx, "checkpoint_rollback", json!({
        "checkpoint_id": checkpoint_id, "reason": body.reason,
        "rolled_back_by": "admin:cli",
    })).await;
    if !out.ok { return Err(ApiError::from_tool_out(out)); }
    Ok((StatusCode::ACCEPTED, Json(RollbackResponse {
        checkpoint_id, rolled_back_at: extract_micros(&out),
    })))
}
```

`ToolContext::for_internal` constructs a `MaskContext` with a mode
that the mask layer treats as internal-trusted — story 1.5 already
exposed `AgentMode::Initializer` for the Initializer's `plan_create`
calls. Either reuse `Initializer` here or add a new `Internal` variant
in 1.5's enum (cheap, defensible). Pick one and document in the
implementation steps; the regression test
`mask_blocks_checkpoint_rollback_from_worker_mode` still holds.

### 4. Gate verifier-rollback path (config-gated)

In `verifier::gate.rs` (story 1.10), extend the `verdict=fail` arm:

```rust
let want_rollback = ev.data.get("rollback_required").and_then(Value::as_bool).unwrap_or(false);
if state.config.checkpoint.rollback_on_verifier_fail && want_rollback {
    if let Some(latest) = state.checkpoints.latest_for_session(&ev.session_id).await {
        let ctx = ToolContext::for_internal(state, &ev.session_id);
        state.dispatcher.dispatch_internal(&ctx, "checkpoint_rollback", json!({
            "checkpoint_id": latest.id, "reason": "verifier_fail",
            "rolled_back_by": "verifier",
        })).await;
    }
}
```

The config defaults `rollback_on_verifier_fail = false`; the path is
present but inactive in Phase 1. phase-1/DEBT.md #3 tracks the
decision to flip the default.

### 5. Config

```toml
[checkpoint]
rollback_on_verifier_fail = false   # phase-1/DEBT.md #3
admin_token_env           = "SEASONED_HAND_ADMIN_TOKEN"
```

### 6. Misc-kind documentation

Append `checkpoint_rollback` to the documented set.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core tools::checkpoint_rollback::
cargo test -p seasoned-hand-core checkpoint::routes::admin_rollback
cargo test -p seasoned-hand-core verifier::gate::tests::rollback_path
./scripts/spec-check.sh
```

Live:

```bash
curl -i -X POST http://127.0.0.1:3000/v1/sessions/<id>/checkpoints/<cp>/rollback \
    -H "X-Seasoned-Hand-Admin-Token: $SEASONED_HAND_ADMIN_TOKEN" \
    -d '{"reason":"manual test"}'
# expect 202; row's rolled_back_at populated.
```

---

## Files changed

- `crates/seasoned-hand-core/src/tools/checkpoint_rollback.rs` (new)
- `crates/seasoned-hand-core/src/tools/registry.rs` (modify — register)
- `crates/seasoned-hand-core/src/checkpoint/persistence.rs` (modify —
  `mark_rolled_back` + `latest_for_session`)
- `crates/seasoned-hand-core/src/checkpoint/routes.rs` (modify — add
  `post_rollback`)
- `crates/seasoned-hand-server/src/state.rs` (modify — `admin_token`
  loaded from env)
- `crates/seasoned-hand-server/src/main.rs` (modify — register admin
  route, wire opt-in config flag)
- `crates/seasoned-hand-core/src/verifier/gate.rs` (modify — opt-in
  rollback branch)
- `crates/seasoned-hand-core/src/dispatch/mask.rs` (modify if a new
  `Internal` variant is preferred over reusing `Initializer`)
- `crates/seasoned-hand-core/src/events/misc.rs` (modify — document
  `checkpoint_rollback`)
- `config/seasoned-hand.toml` (modify — `[checkpoint]` block)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.6 (rollback half — verbatim
  command + rationale), §4.1 (admin endpoint contract), §9 (admin
  auth posture), `phase-1/DEBT.md` #3 (opt-in default rationale).

---

## Commit message

```
feat(phase-1): story 1.13b - checkpoint rollback (internal tool + admin endpoint)

- checkpoint_rollback tool: registered but LLM-masked by story-1.5
  DefaultMaskPolicy (regression test asserts this); runs `git -C
  /workspace revert --no-commit <sha>` inside the sandbox (architecture
  §2.6 explicit choice — preserves history, agent sees a follow-up
  Misc), UPDATEs the row's rolled_back_at/rolled_back_by, emits Misc
  checkpoint_rollback
- POST /v1/sessions/:id/checkpoints/:cp/rollback admin endpoint:
  loopback bind + X-Seasoned-Hand-Admin-Token header; rejects
  401 unauthorized_token / 403 forbidden_non_loopback / 409
  wrong_state (RUNNING|VERIFYING) / 409 sandbox_paused / 503
  admin_token_not_configured; 202 on success
- Opt-in Verifier-driven rollback wired in VerifierGate behind
  config flag checkpoint.rollback_on_verifier_fail (default FALSE
  per phase-1/DEBT.md #3); when off, rollback_required field on
  verdicts is logged but ignored
- 11 unit + integration tests

Debt: phase-1/DEBT.md #3 unchanged — auto-rollback default decision
deferred to Phase 2 retrospective once Verifier precision data
exists.

refs: /specs/phase-1/stories/story-1.13b.md
```

---

## Notes for next story (1.14)

Rollback is fully wired but inactive by default. Story 1.14 (DEBT #21)
is unrelated and remains the next sequential story. The Phase 2
retrospective is when the team decides whether to flip
`rollback_on_verifier_fail = true` based on real Phase 1 Verifier
precision numbers.
