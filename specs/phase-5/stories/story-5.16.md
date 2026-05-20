# Story 5.16 — events::visibility module + admin raw-event route

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 5.14, 5.5
> **Phase**: 5
> **Type**: backend+api

---

## Goal

Add the events::visibility read surface so tenant-visible feeds (user/viewer roles) return the
redacted projection from `tenant_event_view`. Admin-only raw-event route exists for forensics
but is `Action::EventRawRead`-gated and audit-logged.

## Acceptance criteria

- [ ] `crate::events::visibility::query(session_id, ctx)` returns redacted rows from
      `tenant_event_view` filtered by tenant + visibility_level.
- [ ] HTTP route `GET /v1/events/{session_id}` uses visibility query for non-admin roles.
- [ ] HTTP route `GET /v1/admin/events/{session_id}/raw` returns raw `events.data`, but only
      to admin role; every successful read writes an audit_log row.
- [ ] Viewer accessing raw route → 403.

## Verification

```bash
cargo test -p seasoned-hand-server events::visibility_api
```

## Refs

- requirements: F-5.11, F-5.12
- architecture: §7
