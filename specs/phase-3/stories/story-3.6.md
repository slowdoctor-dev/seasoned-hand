# Story 3.6 — Initializer top-3 deterministic playbook injection

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 3.5
> **Phase**: 3
> **Type**: backend

---

## Goal

Inject up to top-3 matched playbooks into Initializer prompt-prefix deterministically,
without extra LLM calls, enforcing aggregate byte cap and truncation telemetry.

## Acceptance criteria

- [ ] Injection happens at task start in Initializer system context.
- [ ] Zero-match path skips silently; 1-2 matches inject only available rows.
- [ ] Aggregate payload cap is enforced and emits `playbook_injection_truncated` when hit.
- [ ] Injection ordering follows deterministic matcher order.
- [ ] No additional LLM round-trip is introduced for injection.

## Non-goals

- Outcome counter updates.

---

## Implementation steps

1. Wire matcher output into Initializer prompt builder.
2. Implement top-3 formatter and byte-cap truncation.
3. Emit `Skill{kind:"injection"}` and truncation `Misc` event when applicable.

---

## Verification

```bash
cargo test -p seasoned-hand-core injector::top3_behavior
cargo test -p seasoned-hand-core injector::byte_cap_and_event
```

---

## Refs

- requirements: F-3.11, NFR-3.2, NFR-3.3
- architecture: §2, §4, §11
