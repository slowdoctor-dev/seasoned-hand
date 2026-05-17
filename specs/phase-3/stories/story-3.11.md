# Story 3.11 — CLI session search with summarized operator view

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 3.8
> **Phase**: 3
> **Type**: cli

---

## Goal

Ship the required `session search` operator surface returning raw FTS hits plus LLM
summary, with deterministic fallback to raw hits on summarizer failure.

## Acceptance criteria

- [ ] `seasoned-hand session search <query>` returns raw hits from session search index.
- [ ] Command also returns summarized query-centric view.
- [ ] Summarizer failure degrades to raw hits without command failure.
- [ ] Degradation path emits operability telemetry event.

## Non-goals

- Browser-side search UI.

---

## Implementation steps

1. Add `session search` command path and output modes.
2. Wire summarization call via configured slot.
3. Add fallback and event emission tests.

---

## Verification

```bash
cargo test -p seasoned-hand-cli commands::session_search
cargo test -p seasoned-hand-core session_search::summary_fallback
```

---

## Refs

- requirements: F-3.17, NFR-3.6
- architecture: §4, §8
- debt context: #75
