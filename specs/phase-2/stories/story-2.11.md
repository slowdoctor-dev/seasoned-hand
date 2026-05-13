# Story 2.11 — EmailChannel (IMAP intake + SMTP delivery + notify)

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 2.4, 2.5
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.7, §2.8 "Email intake", §2.9 "Email delivery", §9 "Email intake authentication"

---

## Goal

One `EmailChannel` struct implementing all three role traits. Email
is the most natural non-technical inbound channel — non-engineers
expect to delegate work via reply-to-an-address.

> **Split risk**: this story is at the 3h budget edge. If implementation
> blows past 3.5h, split into 2.11a (IMAP IntakeProvider) + 2.11b
> (SMTP DeliverySink + NotifySink). The PM session pre-authorized
> the split.

## Acceptance criteria

- [ ] `seasoned_hand_core::channel::email::EmailChannel` struct holds:
      IMAP config (`imap_host`, `imap_port`, `imap_username`,
      `imap_password`), SMTP config (same shape), subject prefix
      (default `[sh]`), allow-list regex Vec.
- [ ] **IntakeProvider impl**: `run()` runs an IMAP poll loop
      (30 s default interval, configurable via `IMAP_POLL_INTERVAL_SEC`).
      Each cycle:
      - Connect via `async-imap` (TLS by default)
      - `SELECT INBOX`
      - `SEARCH UNSEEN`
      - For each new message:
        - Parse via `mailparse`
        - Verify sender matches at least one entry in
          `INTAKE_EMAIL_ALLOWED_SENDERS` (comma-separated regex env;
          deny-all default). Reject + skip if not allowed.
        - Verify subject starts with prefix (default `[sh]`).
        - Construct `IntakeEvent { channel: "email", intake_id:
          format!("imap:{uid}"), brief_input: body_plain,
          reply_target: Some(DeliveryTarget { channel: "email",
          target_ref: format!("msgid:<{msgid}>"), metadata: ... }),
          metadata: { from, subject, has_attachments, spf, dkim } }`
        - Drop attachments into `/workspace/.intake/<intake_id>/` via
          the worker's first session's SandboxClient (resolved post-
          hoc; if no session yet, defer to attachment fetch on first
          task action).
        - Mark message as `\Seen`. Push IntakeEvent to sink.
- [ ] **DeliverySink impl**: `deliver(target, deliverable)` parses the
      `target.target_ref` (`"msgid:<...>"`), uses `lettre` to craft a
      reply email:
      - `In-Reply-To` + `References` headers set to the original
        Message-ID
      - Subject prefixed `[Re: <original-subject>]`
      - Body: short text plus the rendered artifact as an attachment
        (filename = `deliverable.filename`, content-type inferred from
        `format`)
- [ ] **NotifySink impl**: `notify(target, event)` sends a plain
      email to `target.target_ref` (an email address, NOT a
      Message-ID) with subject like `[sh] {trigger_kind}` and a
      JSON-pretty-printed `event.payload` in the body.
- [ ] Allow-list deny default: `INTAKE_EMAIL_ALLOWED_SENDERS=""`
      causes the IntakeProvider to reject every incoming email and
      log `Misc{kind:"intake_sender_rejected", from, reason:"allowlist_empty"}`.
- [ ] SPF / DKIM signal: if `mailparse` exposes Authentication-Results
      header, persist into `metadata.spf` + `metadata.dkim`. Failed
      signatures **don't block** intake automatically in Phase 2
      (operator-configurable later); they're surfaced as Misc events
      for auditability.
- [ ] Unit tests:
      - `email_imap_intake_creates_event_from_test_message` (canned
        IMAP server via `async-imap` test harness)
      - `email_intake_rejects_unknown_sender` (no allow-list match)
      - `email_intake_requires_subject_prefix`
      - `email_delivery_sends_reply_with_attachment` (lettre's
        `StubTransport`)
      - `email_notify_sends_status_message`
      - `email_intake_records_dkim_pass_in_metadata`

## Non-goals

- `text/html` body parsing — Phase 2 uses `text/plain` only; html-only
  emails get rejected with `intake_no_plain_body`.
- Multi-mailbox support (Phase 4+ would let multiple `@example.com`
  addresses route to different projects).
- OAuth-based IMAP/SMTP (Phase 5 multi-user; env-based password for
  Phase 2).

---

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/channel/email/
  mod.rs          ← EmailChannel + 3 trait impls
  imap.rs         ← poll loop + mailparse helpers
  smtp.rs         ← lettre helpers
  allowlist.rs    ← sender regex matching
  tests.rs
```

### 2. Dependencies

Add to `crates/seasoned-hand-core/Cargo.toml`:
- `lettre = { version = "0.11", default-features = false, features = ["tokio1-rustls-tls", "smtp-transport", "builder"] }`
- `mailparse = "0.15"`
- `async-imap = { version = "0.10", default-features = false, features = ["runtime-tokio"] }`

### 3. Polling lifecycle

`run()` runs `loop { tokio::select! { _ = shutdown.cancelled() => return Ok(()), _ = tokio::time::sleep(interval) => self.poll_once(&sink).await }`. Errors in `poll_once` are logged + counted toward
exponential backoff up to a 5-min cap.

### 4. Reply construction

`In-Reply-To: <original-msgid>` + `References: <original-msgid>`.
Email clients thread the response. Subject collision detection: if
the original Subject already starts with `[Re: ...]`, don't double.

### 5. Tests

`async-imap` exposes a `Server` test fixture. Use it to plant a known
message and verify our poller picks it up. `lettre::transport::stub::StubTransport`
captures sent emails for assertion.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core channel::email
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/channel/email/{mod,imap,smtp,allowlist,tests}.rs` (new)
- `crates/seasoned-hand-core/src/channel/mod.rs` (modify)
- `crates/seasoned-hand-core/Cargo.toml` (modify — add 3 deps)
- `crates/seasoned-hand-server/src/lib.rs` (modify — register EmailChannel
  when SMTP_HOST + IMAP_HOST env present; skip cleanly when absent)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.7, §2.8, §2.9, §9
- `/specs/phase-2/DEBT.md` #4 (operator-curated allow-list)

---

## Commit message

```
feat(phase-2): story 2.11 - EmailChannel (IMAP intake + SMTP delivery + notify)

One struct, three trait impls.

- IntakeProvider: 30s IMAP poll cycle. Allow-list regex + subject
  prefix gate. Per-message metadata captures from / subject /
  has_attachments / spf / dkim. Attachments dropped into
  /workspace/.intake/<intake_id>/.
- DeliverySink: lettre SMTP reply to In-Reply-To header. Subject
  prefixed [Re: ...]. Rendered artifact attached.
- NotifySink: plain status email to target address, subject [sh]
  <trigger_kind>.
- Deps: lettre 0.11 (SMTP), mailparse 0.15 (RFC 5322), async-imap
  0.10 (poller). All tokio-rustls.
- 6 unit tests.

refs: /specs/phase-2/stories/story-2.11.md
```

---

## Notes for next story (2.12)

EmailChannel is the largest channel story. NtfyChannel (2.12) is
notify-only — much smaller. 2.12 also bundles NotifyWorker (consumes
notify_request Redis stream → dispatches to registered NotifySink
channels).
