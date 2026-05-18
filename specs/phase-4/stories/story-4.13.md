# Story 4.13 — Auto-archive policy engine + reversibility

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 4.5, 4.9, 4.12
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Implement archive recommendation/apply/restore semantics with project-level thresholds and archive
reason/confidence metadata.

## Acceptance criteria

- [ ] Auto-archive recommendations and apply path are implemented.
- [ ] Restore/unarchive path preserves provenance and ranking metadata.
- [ ] Archive metadata (`archived_reason`, confidence context) is persisted.
- [ ] Integration test covers archive + restore roundtrip.

## Non-goals

- Adaptive thresholds.

---

## Implementation steps

1. Implement threshold policy and action routing.
2. Implement restore path and metadata persistence.
3. Add archive/restore regression tests.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.8, F-4.9, NFR-4.7
- architecture: §3.2, §12.1, §12.11
- debt closed: #90 (close), #92 (close)
