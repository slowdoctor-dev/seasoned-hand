# Story 2.10 — WebhookChannel (intake + delivery + notify)

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 2.4, 2.5
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.7, §2.8 "Webhook intake", §2.9 "Webhook delivery", §9 "Webhook intake authentication"

---

## Goal

One `WebhookChannel` struct implementing all three role traits. The
minimum-viable OS surface — any external system speaks HTTP to us and
gets HTTP back.

## Acceptance criteria

- [ ] `seasoned_hand_core::channel::webhook::WebhookChannel` struct
      holds: `intake_token: Arc<String>` (from `SEASONED_HAND_INTAKE_TOKEN`
      env), `http: reqwest::Client`.
- [ ] **IntakeProvider impl**: `run()` registers the
      `POST /v1/intake/webhook` axum route handler. The handler:
      - Verifies `X-Seasoned-Hand-Intake-Token` header matches
        `intake_token` (constant-time compare). Empty token →
        `503 intake_token_not_configured`. Wrong token → `401`.
      - Parses body: `{brief: String, project_id?: String,
        reply_target?: DeliveryTarget, metadata?: object}`.
      - Constructs `IntakeEvent { channel: "webhook", intake_id: req_id,
        ... }` and pushes to sink.
      - Returns `202 Accepted` with `{task_id, briefing_call_id}`.
- [ ] **DeliverySink impl**: `deliver(target, deliverable)` POSTs to
      `target.target_ref` (a callback URL) the JSON `{task_id,
      deliverable_id, format, content_url, provenance_manifest,
      status}`. `content_url` is the server-relative
      `/v1/tasks/:id/deliverables/:did/content`. The callback fetches
      the bytes itself.
- [ ] **NotifySink impl**: `notify(target, event)` POSTs to
      `target.target_ref` the JSON `{task_id?, trigger_kind, payload}`.
- [ ] **SSRF protection** in `deliver` + `notify`: the target URL is
      DNS-resolved before POST; if any A/AAAA record is in
      private/link-local/loopback range, reject with
      `ChannelError::RemoteRejected { code: 400, message:
      "private_address_rejected" }`. Operator allow-list
      (`WEBHOOK_DELIVERY_ALLOWLIST` env, comma-separated CIDRs) bypasses.
      phase-2/DEBT.md #1 already tracks this as permissive-by-default
      for Phase 2.
- [ ] Token auth: `intake_token` field is `Arc<String>` populated
      from `SEASONED_HAND_INTAKE_TOKEN` env at boot. Empty token
      means the intake endpoint is **disabled** (503).
- [ ] Registered at `AppState::new`:
      ```rust
      let webhook = Arc::new(WebhookChannel::new(intake_token.clone()));
      state.channels.register(
          ChannelRegistration::new("webhook")
              .with_intake(webhook.clone())
              .with_delivery(webhook.clone())
              .with_notify(webhook),
      );
      ```
- [ ] Unit tests:
      - `webhook_intake_creates_event_and_returns_task_id`
      - `webhook_intake_rejects_without_token`
      - `webhook_intake_returns_503_when_token_unset`
      - `webhook_delivery_posts_callback`
      - `webhook_delivery_retries_5xx_via_router` (covered in 2.5;
        this story asserts the Err variant is `Http(5xx)`)
      - `webhook_delivery_rejects_private_ip`
      - `webhook_delivery_allows_private_ip_with_allowlist`
      - `webhook_notify_posts_to_target`

## Non-goals

- Multi-user webhook auth (Phase 5).
- HMAC signatures on incoming webhooks (Phase 4 if requested).
- Per-tenant token (Phase 5).
- The retry policy is in DeliveryRouter (story 2.5), not here.

---

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/channel/webhook/
  mod.rs            ← WebhookChannel + 3 trait impls
  intake_handler.rs ← axum POST /v1/intake/webhook handler
  ssrf.rs           ← DNS resolution + private-IP check
  tests.rs
```

### 2. Constant-time token compare

Use `subtle::ConstantTimeEq` (already a transitive dep in workspace).
Same pattern as Phase 1 1.13b admin-rollback.

### 3. SSRF protection

```rust
async fn assert_public_address(url: &Url, allowlist: &[IpNet]) -> Result<(), SsrfRejection> {
    let host = url.host_str().ok_or(...)?;
    let addrs = tokio::net::lookup_host(format!("{host}:0")).await?;
    for addr in addrs {
        if !is_public(addr.ip()) && !in_allowlist(addr.ip(), allowlist) {
            return Err(SsrfRejection::private(addr.ip()));
        }
    }
    Ok(())
}
```

Uses `ipnet` crate (transitive dep check; if not present, add as new
Phase-2 dep alongside lettre).

### 4. Route registration

The `IntakeProvider::run` hook returns a `Router` that the server
mounts via `app(state)`'s Router builder. Architecture deviation from
the "long-lived loop" framing: webhook intake is HTTP-driven, not
polling. `run()` returns immediately; the route handler is the actual
intake source. Documented in this story's execution notes (if needed
post-implementation).

Actually cleaner: WebhookChannel does NOT use `run()`; the intake
route is registered by `AppState::new` directly, and the channel's
`IntakeProvider::run` is a no-op (same pattern as ChatChannel — the
intake source is external to `run()`). The route handler still
constructs IntakeEvents through the channel.

### 5. Tests

Wiremock the outbound HTTP for delivery + notify. axum test client
for inbound intake (mirror Phase 1 admin_rollback.rs pattern).

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core channel::webhook
cargo test -p seasoned-hand-server --test webhook_intake
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/channel/webhook/{mod,intake_handler,ssrf,tests}.rs` (new)
- `crates/seasoned-hand-core/src/channel/mod.rs` (modify — `pub mod webhook;`)
- `crates/seasoned-hand-server/src/lib.rs` (modify — register WebhookChannel,
  mount `POST /v1/intake/webhook`)
- `crates/seasoned-hand-server/tests/webhook_intake.rs` (new integration
  test)
- Possibly `crates/seasoned-hand-core/Cargo.toml` — add `ipnet` if not
  present

---

## Spec references

- `/specs/phase-2/architecture.md` §2.7 (channel matrix),
  §2.8 (intake protocol — webhook), §2.9 (delivery — webhook),
  §9 (auth, SSRF)
- `/specs/phase-2/DEBT.md` #1 (SSRF permissive default)

---

## Commit message

```
feat(phase-2): story 2.10 - WebhookChannel (intake + delivery + notify)

One struct, three trait impls. The minimum-viable OS surface — any
external system speaks HTTP to us.

- IntakeProvider: POST /v1/intake/webhook handler. Token auth via
  X-Seasoned-Hand-Intake-Token (constant-time compare). Empty
  SEASONED_HAND_INTAKE_TOKEN env disables (503). Constructs
  IntakeEvent and pushes to IntakeRouter; returns 202 + {task_id}.
- DeliverySink: POST to callback URL with task_id + content_url +
  provenance_manifest. Body fetchable via /v1/tasks/:id/deliverables/
  :did/content.
- NotifySink: POST to target URL with {task_id?, trigger_kind, payload}.
- SSRF protection: DNS-resolve target URLs; reject if any resolved
  IP is private / link-local / loopback. WEBHOOK_DELIVERY_ALLOWLIST
  env bypasses. Tracked as phase-2/DEBT #1 (permissive-by-default
  for Phase 2 single-user).
- 8 unit + 1 integration test.

refs: /specs/phase-2/stories/story-2.10.md
```

---

## Notes for next story (2.11)

Webhook is in. 2.11 ships EmailChannel — bigger surface (IMAP intake
poller via async-imap + mailparse, SMTP delivery + notify via lettre).
Same one-struct pattern. EmailChannel is the largest single-channel
story; watch for the 3h budget.
