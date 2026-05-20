# Story 5.14 — tenant_event_view projection + write-time redaction hook

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 5.2
> **Phase**: 5
> **Type**: backend+security

---

## Goal

Implement the tenant-safe event projection per architecture §7. Every event written to
`events` triggers a write-time hook that redacts payload via `redact_pii` (extended with
tool-arg scrub patterns) and inserts a row in `tenant_event_view`. Closes the load-bearing
SECURITY_REVIEW iter-3 carry-forward (DEBT #S-1).

## Acceptance criteria

- [ ] `crate::events::visibility::RedactionHook` runs on every `events.append`.
- [ ] Successful redaction → `tenant_event_view` row inserted with `redacted_data`,
      `searchable_text`, `visibility_level` per arch §7.
- [ ] Failed redaction → row skipped + `Misc{kind:"tenant_event_projection_failed"}` emitted
      (quarantine semantics).
- [ ] Action/Observation events containing PEM keys / IPv6 forms / Authorization headers (the
      patterns from `crate::verifier::extraction::redact_pii`) emerge fully redacted in
      `tenant_event_view`.

## Verification

```bash
cargo test -p seasoned-hand-core events::visibility::tests
```

## Refs

- requirements: F-5.11, F-5.12, NFR-5.6
- architecture: §7, §7.1
- debt closed: #S-1 (close)
