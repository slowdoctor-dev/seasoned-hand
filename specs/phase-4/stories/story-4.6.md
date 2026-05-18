# Story 4.6 — ConflictDetector production implementation

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 4.2, 4.4
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Ship production ConflictDetector using structural-step diff + semantic contradiction adjudication.

## Acceptance criteria

- [ ] Production ConflictDetector exists (not trait-only/test-only scaffold) and writes
      `sop_conflicts` rows.
- [ ] Baseline algorithm follows F-4.10 requirement and architecture §12.9.
- [ ] Severity triage path implemented.
- [ ] Runtime wiring is enabled from `main.rs` via CuratorWorker dependency graph.
- [ ] Integration test covers conflict/open + non-conflict paths with stub semantic adjudication in
      end-to-end curator cycle execution.

## Non-goals

- Retrospective generation.

---

## Implementation steps

1. Implement structural prefilter and semantic adjudication.
2. Implement severity classification and persistence.
3. Add integration tests with deterministic fixtures.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.10, F-4.11, NFR-4.5
- architecture: §2.5, §3.2, §4.4, §12.9
- debt closed: #89 (partial)
