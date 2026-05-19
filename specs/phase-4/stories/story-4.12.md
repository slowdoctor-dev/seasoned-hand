# Story 4.12 — Curator telemetry schema + event taxonomy reconciliation

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 4.2, 4.3, 4.5, 4.6, 4.7, 4.8
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Emit Phase 4 telemetry (`curator_*` Misc) and `Skill{kind:"curation_decision"}` consistently,
with event taxonomy reconciliation checks.

## Acceptance criteria

- [ ] Curator telemetry events from architecture §4.5 are emitted.
- [ ] `Skill.kind=curation_decision` events are emitted and indexed.
- [ ] Event taxonomy validation test protects shape/name drift.

## Non-goals

- Dashboard implementation.

---

## Implementation steps

1. Add event emitters across runtime components.
2. Add session_search indexing coverage for new kinds.
3. Add taxonomy regression tests.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.15, F-4.16
- architecture: §4.5, §6.4
- debt closed: #88 (close)
