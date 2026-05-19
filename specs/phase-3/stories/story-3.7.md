# Story 3.7 — Skill telemetry outcomes and playbook counters

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 3.6
> **Phase**: 3
> **Type**: backend

---

## Goal

Record `Skill` outcome events and maintain `success_count` / `failure_count` from verifier
verdicts for injected playbooks, while preserving Phase 4 curator boundaries.

## Acceptance criteria

- [ ] `pass` verdict increments `success_count` and emits `Skill{kind:"outcome"}`.
- [ ] `fail` verdict increments `failure_count` and emits `Skill{kind:"outcome"}`.
- [ ] Other verdicts do not update counters and emit no outcome event.
- [ ] Auto-extracted playbooks are immediately eligible (`status='active'`).
- [ ] No archive/consolidate/rate-threshold policy is implemented.

## Non-goals

- Curator automation decisions.

---

## Implementation steps

1. Read task injection set from events stream.
2. Update playbook counters transactionally with outcome events.
3. Guard non-pass/fail verdict behavior.

---

## Verification

```bash
# Phase 4 manageability hardening (commit e004b2d) deleted the empty
# `crate::skill` module; these tests live under `verifier::gate::tests`:
cargo test -p seasoned-hand-core verifier::gate::tests::outcome_counter_updates
cargo test -p seasoned-hand-core verifier::gate::tests::non_terminal_verdict_noop
```

---

## Refs

- requirements: F-3.8, F-3.9, F-3.15
- architecture: §4, §6
- debt closure: partial paydown of Phase 2 DEBT #61 (`Skill` writer)
