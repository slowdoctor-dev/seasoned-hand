# Story 3.14 — Regression: FTS5 maintenance trigger correctness

> **Status**: done
> **Estimated**: 1 hour
> **Dependencies**: 3.2, 3.8
> **Phase**: 3
> **Type**: test

---

## Goal

Verify FTS5 external-content maintenance triggers remain correct for both playbook and
session-search indexes across insert/update/delete paths.

## Acceptance criteria

- [x] Integration tests cover `playbooks_ai/ad/au` behavior.
- [x] Integration tests cover `session_search_index_ai/ad/au` behavior.
- [x] Trigger-backed index contents stay consistent with source rows.

## Non-goals

- Search ranking semantics.

---

## Implementation steps

1. Add trigger-focused integration tests for playbooks and session search.
2. Assert index consistency after each DML operation.

---

## Verification

```bash
cargo test -p seasoned-hand-core fts5::trigger_correctness
```

---

## Refs

- requirements: F-3.16
- architecture: §3 (trigger definitions), §11
