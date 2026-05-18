# Story 4.5 — ConsolidationEngine with revision-chain writes

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 4.2, 4.4
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Ship production ConsolidationEngine to produce/apply merge/keep/archive/quarantine decisions and
write revision-chain lineage consistently.

## Acceptance criteria

- [ ] Production ConsolidationEngine exists (not trait-only/test-only scaffold).
- [ ] Consolidation decision policy implemented per architecture §12.2/§12.3.
- [ ] Revision-chain write path updates `playbook_revisions` + active revision pointers.
- [ ] Low-confidence decisions are queued for review.
- [ ] Runtime wiring is enabled from `main.rs` via CuratorWorker dependency graph.
- [ ] Integration test covers merge and keep branches with stub rerank input in end-to-end curator
      cycle execution.

## Non-goals

- Review queue CLI.

---

## Implementation steps

1. Implement decision policy engine.
2. Implement transactional apply path.
3. Add integration tests for merge/keep/quarantine.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.6, F-4.7, F-4.8, F-4.9
- architecture: §2.4, §3.2, §4.3, §12.2, §12.3
- debt closed: #90 (partial)
