# Story 5.11 — Hand-off audit emission + handoff CLI polish

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 5.9, 5.10
> **Phase**: 5
> **Type**: backend+cli

---

## Goal

Wire the audit_log writer (5.10) into the hand-off path (5.9) so every reassignment leaves an
immutable record. Polish the CLI handoff UX.

## Acceptance criteria

- [ ] Every `task.handoff()` call writes one audit_log row with
      `action='task.handoff', resource_type='task', actor_user_id, target_user_id, reason`.
- [ ] CLI confirms successful handoff with the audit_log row id printed (operator can reference
      it later).
- [ ] `seasoned-hand audit list --task <id>` filter shows hand-off history per task.

## Verification

```bash
cargo test -p seasoned-hand-core handoff::audit_integration
```

## Refs

- requirements: F-5.8, F-5.9, NFR-5.3
- architecture: §5, §4.3
