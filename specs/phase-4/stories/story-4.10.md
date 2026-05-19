# Story 4.10 — Knowledge/Datasource writers + L2 enforcement rollout

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 4.2, 4.3, 4.4
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Ship production Knowledge/Datasource write paths and phased L2 enforcement with feature flags.

## Acceptance criteria

- [ ] Production Knowledge/Datasource writer runtime exists (not trait-only/test-only scaffold).
- [ ] Raw staging writes for knowledge/datasource are implemented.
- [ ] Canonical promotion path follows L2 rules from architecture §12.15.
- [ ] Feature-flagged rollout by artifact class is implemented.
- [ ] Runtime wiring is enabled from `main.rs` via CuratorWorker dependency graph.
- [ ] Integration test covers emit and promotion scenarios in end-to-end curator cycle execution
      with stub external services.

## Non-goals

- Cross-project analytics.

---

## Implementation steps

1. Implement raw emit conditions and table writers.
2. Implement L2 promotion policy engine.
3. Add rollout flags and integration tests.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.12, F-4.13
- architecture: §3.2, §12.14, §12.15
- debt closed: #73 (close)
