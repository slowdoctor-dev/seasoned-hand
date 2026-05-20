# Observability Review Log

Cross-phase audit trail for codebase observability — log structure,
error context preservation, worker visibility, and silent-failure
detection. Mirrors `SECURITY_REVIEW.md` + `MANAGEABILITY_REVIEW.md`.

---

## Audit cycle — 2026-05-20 (Claude solo)

> Reviewer: Claude solo (Codex on 5-day rate-limit recovery)
> Scope: post-Phase-4-close-out observability sweep
> Method: tracing-call-site map → error-context analysis → silent-error
> probe → fix commits → saturation re-sweep

### Surfaces probed

| Surface | Verdict |
|---|---|
| Tracing level distribution (error/warn/info/debug/trace) | 25 / 128 / 28 / 4 / 0 — sane shape |
| Tracing macros without structured fields | 0 |
| Healthz endpoint coverage | db + redis liveness probes (operator-acceptable) |
| Metrics surface (Prometheus / OpenTelemetry) | none — Phase 4 uses `curator_*` event-stream taxonomy as the observability layer (accepted for the single-operator-local design) |
| `#[instrument]` / `tracing::span!` for span hierarchy | 0 — accepted; the codebase uses kv-fields on individual log lines rather than span context |
| `TODO` / `FIXME` markers signalling open obs work | 0 |

### F1 (M) — curator + retention worker logs lacked `project_id`

**Status**: FIXED at commit `b873119` (observability iter-1)

**Threat model**: 6 log sites in CuratorWorker::run and
RetentionScheduler::run emitted without `project_id` context despite
the surrounding scope holding `self.config.project_id`. In multi-
project Phase 5 deployments, "curator cycle failed" would have been
unattributable; even today, an operator setting `SH_CURATOR_PROJECT_ID`
away from "default" can't grep logs by project.

**Fix**: each `run` captures `project_id` once at the top and threads
it into every emit. `curator cycle failed` also picks up `trigger` +
`backlog` fields for better post-mortem.

### F2 (M) — asymmetric worker spawn logs

**Status**: FIXED at commit `516cef7` (observability iter-2)

**Threat model**: Curator + Verifier `else` branches emitted "X not
spawned (FLAG=false)" but the active branches were silent. Retention
scheduler was silent on both branches. An operator reading boot logs
had no positive signal that the optional workers came up successfully.

**Fix**: symmetric info-level spawn logs:
- curator: `project_id`, `interval_seconds`, `backlog_threshold`,
  `auto_archive_enabled`
- verifier: `learning_enabled`, `rollback_on_fail`
- retention: `project_id`, `interval_seconds`

### F3 (M) — `map_err(|_| LlmError::MissingChoice)` masked parse failures

**Status**: FIXED at commit `516cef7` (observability iter-2)

**Threat model**: `Initializer::call_planner_slot` mapped any
`serde_json::Error` from parsing the LLM response content to
`LlmError::MissingChoice`. Two wrong things at once: the variant
lies (the choice IS present), and the parse error's line/column
diagnostic was discarded. JSON-shape regressions in the LLM response
would have been very hard to debug.

**Fix**: replaced with `?` — `LlmError::JsonParse(#[from] serde_json::Error)`
already exists, so the From-conversion preserves line/column.

### F4 (M) — HTTP handlers swallowed root-cause errors

**Status**: FIXED at commit `a02cef3` (observability iter-3)

**Threat model**: 3 handler sites in `seasoned-hand-server/src/lib.rs`
mapped real I/O / parse errors to opaque API responses without logging
the underlying error first. API response correctly stayed opaque, but
the operator's log stream lost the actionable detail.

**Fix**: each `map_err(|_| ...)` became `map_err(|error| { warn!(...); ... })`:
- `get_feature_list` workspace-read failure now logs `session_id` +
  `%error`
- `get_feature_list` JSON parse failure now logs `session_id` +
  `error.line()` + `error.column()` + `%error`
- `get_progress` workspace-read failure now logs `session_id` +
  `%error`
- Webhook intake JSON body parse failure now logs `%error`

The `list_events::EventType::from_str` map_err is left as-is — the
underlying error is just "unknown enum variant" and the raw string
is already implicit in the request URL.

### F5 (M) — ws spawn silently dropped runner.resume failure

**Status**: FIXED at commit `6702260` (observability iter-4)

**Threat model**: ws.rs `task_resume` command spawned
`runner.resume(...)` in the background and discarded its result via
`let _ = runner.resume(...).await;`. The user-facing Ack was sent
unconditionally, so if resume actually failed (sandbox unreachable,
db error), the WS client thought the task was running while nothing
was. No log fired.

**Fix**: `if let Err(error) = runner.resume(...).await { tracing::warn!(session_id, %error, "ws: spawned runner.resume() failed"); }`.

### Iter-5 — saturation sweep

Probed:
- Remaining `let _ = ....await` patterns in production: all confined
  to test blocks (verifier/gate.rs poll_once calls) or
  receiver-already-dropped sends (ws.rs tx_clone) where logging would
  be noise.
- `if let Err(_)` silent patterns: 0 in production.
- Curator emitted-events vs traced logs: all 7 `curator_*` Misc kinds
  + the `curation_decision` Skill kind carry full structured payloads.
- Tracing subscriber config: kept the default `tracing_subscriber::fmt()`
  with `EnvFilter` — the default already includes target (module path).
  Adding `with_file(true).with_line_number(true)` would help in some
  cases but is marginal for the current single-binary deployment.

**Findings**: zero new code fixes.

### Saturation verdict

Five iterations of fixes (4 M-severity + 1 doc commit), sixth-iteration
sweep found zero load-bearing items. Observability hardening loop
saturates here.

Net behavior change across the cycle:
- 6 worker log sites now carry `project_id` for multi-project attribution
- 3 worker boot sites now log positive spawn signals with key config knobs
- 4 HTTP handler error paths now log root cause before responding
- 1 silent spawned task now logs failure
- 1 lossy `map_err` chain now preserves serde line/column

Codex review of this audit trail can land once the 5-day rate-limit
recovers; the loop is closed from Claude's side.
