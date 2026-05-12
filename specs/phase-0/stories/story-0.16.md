# Story 0.16 — Cost cap (Bifrost /cost polling + per-session DB tracking)

> **Status**: ready
> **Estimated**: 1 hour
> **Dependencies**: story 0.11 (LLM client), 0.14 (agent runner)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §4.4 (cost-tracking ownership), §7 (cost budget), §8 (cost-cap failure mode), `/specs/00-philosophy/PRINCIPLES.md` #10 (failure-tolerant)

---

## Goal

Replace story 0.14's no-op `cost_cap_cents` argument with a real
check. After every tool dispatch in the agent loop, poll Bifrost's
`/cost` endpoint, increment the session's `cost_cents` in SQLite,
and halt the session with `Misc{kind:"cost_cap"}` if the cap is
exceeded.

Phase 0 caps at the session level. Per-day caps + cost-attribution
to a specific tool call land in Phase 1.

## Acceptance criteria

- [ ] `seasoned-hand-core::cost` module:
      - `CostClient` wraps `LlmClient` (or a bare reqwest::Client) and
        polls Bifrost's `GET /cost` endpoint
      - `CostSnapshot { total_cents: i64, currency: "USD", ts: i64 }`
      - `async fn snapshot(&self) -> Result<CostSnapshot, CostError>`
      - `async fn delta_cents(&self, baseline: &CostSnapshot) -> Result<i64, CostError>`
        — returns the difference between current and baseline
- [ ] `LlmClient::get_cost()` helper (or just expose a generic
      `get_json<T>(path)` so cost lives alongside chat/models)
- [ ] `AgentRunner` integration:
      - Snapshot Bifrost cost at task start; store as `cost_baseline`
      - After each tool dispatch, snapshot again; compute delta;
        `UPDATE sessions SET cost_cents = cost_cents + delta WHERE id = ?`
      - If `cost_cap_cents` set AND `session.cost_cents >= cap` →
        emit `Misc{kind:"cost_cap", current_cents, cap_cents}`,
        set session state SUSPENDED, return `RunResult{completed:false}`
- [ ] Failure tolerance (PRINCIPLE #10):
      - If `/cost` poll fails (Bifrost down, parse error), log
        `tracing::warn`, increment cost by 0 for that step, continue
        the loop. **The agent run does not abort on cost-poll failure.**
- [ ] HTTP route `GET /v1/cost` proxies to Bifrost so the frontend
      (Phase 0.21+) can show current cost
- [ ] Unit tests via wiremock:
      - `delta_cents_returns_positive_diff`
      - `cost_poll_failure_returns_err_caller_tolerates`
      - `agent_runner_halts_on_cost_cap`
      - `agent_runner_continues_when_cost_poll_fails`
- [ ] DEBT.md: close 0.14's "cost cap is no-op" entry
- [ ] `cargo clippy / fmt / test / spec-check` all pass

## Non-goals

- Per-tool-call cost attribution (Phase 1)
- Per-day or per-user caps (Phase 1+; current `cost_cap_cents` is per-run)
- Push-based cost callback from Bifrost (DEBT #11 — Phase 1+)
- Cost prediction before running (Phase 4+; this is observational only)

---

## Implementation steps

### 1. `cost` module

```rust
// crates/seasoned-hand-core/src/cost/mod.rs
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CostError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("status {code}: {body}")]
    Status { code: u16, body: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostSnapshot {
    /// Cumulative cost in US cents (i64 to allow > $21M lifetime).
    /// Bifrost's `/cost` response is typically a float USD; we convert.
    pub total_cents: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub ts: i64,
}

fn default_currency() -> String { "USD".into() }

pub struct CostClient {
    http: reqwest::Client,
    base_url: String,
}

impl CostClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self { http: reqwest::Client::new(), base_url: base_url.into() }
    }

    pub async fn snapshot(&self) -> Result<CostSnapshot, CostError> {
        let url = format!("{}/cost", self.base_url.trim_end_matches('/'));
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            return Err(CostError::Status {
                code: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        // Bifrost v1.5.0 `/cost` returns a JSON object with various fields.
        // Field name unknown until live verification; tolerate both
        // {total_usd: f64} and {total_cents: i64}.
        let v: serde_json::Value = serde_json::from_slice(&bytes)?;
        let total_cents = if let Some(c) = v.get("total_cents").and_then(|x| x.as_i64()) {
            c
        } else if let Some(u) = v.get("total_usd").and_then(|x| x.as_f64()) {
            (u * 100.0).round() as i64
        } else {
            0
        };
        Ok(CostSnapshot {
            total_cents,
            currency: v.get("currency").and_then(|s| s.as_str()).unwrap_or("USD").to_string(),
            ts: now_unix(),
        })
    }

    pub async fn delta_cents(&self, baseline: &CostSnapshot) -> Result<i64, CostError> {
        let now = self.snapshot().await?;
        Ok((now.total_cents - baseline.total_cents).max(0))
    }
}

fn now_unix() -> i64 { /* ... */ }
```

### 2. Agent runner integration

```rust
let cost = CostClient::new(self.bifrost_base_url.clone());
let baseline = cost.snapshot().await.unwrap_or_else(|e| {
    tracing::warn!(error=%e, "cost baseline poll failed; defaulting to 0");
    CostSnapshot { total_cents: 0, currency: "USD".into(), ts: now_unix() }
});

for step in 0..req.max_steps {
    /* dispatch tool ... */

    if let Ok(delta) = cost.delta_cents(&baseline).await {
        self.bump_session_cost(&req.session_id, delta).await.ok();
    } else {
        tracing::warn!("cost delta poll failed; skipping step accounting");
    }

    if let Some(cap) = req.cost_cap_cents {
        let current = self.session_cost(&req.session_id).await.unwrap_or(0);
        if current >= cap as i64 {
            self.emit_misc_with("cost_cap", json!({
                "current_cents": current, "cap_cents": cap
            })).await?;
            self.set_session_state(&req.session_id, "SUSPENDED").await?;
            return Ok(RunResult { /* completed:false ... */ });
        }
    }
}
```

The `baseline` is for the WHOLE Bifrost cost, not just this session.
Polling at task start, then re-polling and taking the delta against
baseline, accumulates only the cost incurred during this run.
(Other sessions on the same Bifrost will also contribute deltas if
they run concurrently — Phase 0 has 1 user / 1 session at a time so
this approximation is fine; DEBT entry covers the concurrency case.)

### 3. HTTP route

`GET /v1/cost` returns `state.cost.snapshot().await`. 503 on poll
failure.

### 4. `AppState` wiring

Add `cost: Arc<CostClient>` field; build in `AppState::new`.

### 5. Tests

Via wiremock for `/cost` responses. Patch the `AgentRunner` test
fixture to inject a controllable cost client.

---

## Files changed

- `crates/seasoned-hand-core/src/lib.rs` (`pub mod cost`)
- `crates/seasoned-hand-core/src/cost/mod.rs` (new)
- `crates/seasoned-hand-core/src/cost/tests.rs` (new)
- `crates/seasoned-hand-core/src/agent/mod.rs` (integrate cost loop)
- `crates/seasoned-hand-core/src/agent/tests.rs` (cost-cap test)
- `crates/seasoned-hand-server/src/lib.rs` (AppState.cost + /v1/cost route)
- `crates/seasoned-hand-server/src/main.rs` (build CostClient)
- `crates/seasoned-hand-server/tests/healthz.rs` + `events.rs` (update construction)
- `specs/phase-0/DEBT.md` (close 0.14 cost-cap entry; add concurrency note)

---

## Spec references

- `/specs/phase-0/architecture.md` §4.4 (cost-tracking ownership across stories)
- `/specs/phase-0/architecture.md` §7 (cost budget $1/session default)
- `/specs/phase-0/architecture.md` §8 (cost_cap failure mode)
- `/specs/00-philosophy/PRINCIPLES.md` #10 (failure-tolerant — cost poll failure does not abort)

---

## Commit message

```
feat(phase-0): story 0.16 - cost cap with Bifrost /cost polling

- seasoned-hand-core::cost::CostClient polls Bifrost /cost,
  tolerates {total_cents} and {total_usd} response shapes
- AgentRunner: snapshot baseline at task start, delta after each
  tool dispatch, accumulate into sessions.cost_cents, halt with
  Misc{kind:"cost_cap"} when threshold reached
- /cost poll failures log warn but do NOT abort the run
  (PRINCIPLE #10 failure-tolerant)
- GET /v1/cost proxies snapshot to the frontend
- AppState gains cost: Arc<CostClient>
- N tests via wiremock: delta calc, poll failure tolerance, cap halt
- cargo clippy / fmt / test / spec-check all pass

Debt: closes 0.14's "cost_cap_cents arg is no-op" entry; adds note
about Phase 0 single-session assumption (concurrent sessions on
same Bifrost would contaminate delta).

refs: /specs/phase-0/stories/story-0.16.md
```
