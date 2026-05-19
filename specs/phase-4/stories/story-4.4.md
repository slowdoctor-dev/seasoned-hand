# Story 4.4 — CandidateBuilder + EmbeddingReranker production wiring

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 4.2, 4.3
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Implement CandidateBuilder and EmbeddingReranker in production, including embedding slot wiring,
blended scoring, cache, and NFR-4.6 budget circuit behavior.

## Acceptance criteria

- [x] Production CandidateBuilder exists (not trait-only/test-only scaffold).
- [x] Production EmbeddingReranker exists (not trait-only/test-only scaffold) and calls Bifrost
      embeddings endpoint with configured model.
- [x] Blend formula + lexical fallback formula match architecture §4.2.
- [x] LRU cache and budget accounting are implemented.
- [x] Runtime wiring is enabled from `main.rs` via CuratorWorker dependency graph.
- [x] Integration test exercises embedding-enabled and embedding-fallback paths with stubbed
      embedding backend in end-to-end curator cycle.

## Non-goals

- Merge/apply decisions.

---

## Implementation steps

1. Implement candidate SQL prefilter + tie-break sort.
2. Wire embedding call contract and caching.
3. Add cost accounting hooks for soft/hard breaker thresholds.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.4, F-4.5, NFR-4.6
- architecture: §2.2, §2.3, §4.2, §7, §12.2, §12.13
- debt closed: #72 (close), #99 (close)
