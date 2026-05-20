# Story 5.24 — FTS5 weight retune (closes/partial DEBT #76)

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 5.15
> **Phase**: 5
> **Type**: backend+tuning

---

## Goal

Per F-5.19 + DEBT #76: retune FTS5 column weights for `playbooks_fts` + `session_search_fts`
+ `curator_search_fts` using Phase 4 dogfood data captured in the warm-loop benchmark.

## Acceptance criteria

- [ ] Measurement procedure documented (eval set + relevance metric).
- [ ] New weights land in the FTS5 `bm25(...)` calls or as column-weight constants.
- [ ] Benchmark from story 4.21 reruns and shows precision@3 stable-or-improved at warm.
- [ ] If dogfood data is insufficient to converge full retune, partial close with explicit
      next-action and successor pointer (Phase 6 if needed).

## Verification

```bash
cargo test -p seasoned-hand-core curator::phase4_warm_full_loop_benchmark
```

## Refs

- requirements: F-5.19
- debt closed: #76 (close or partial)
