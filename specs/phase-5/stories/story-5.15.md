# Story 5.15 — session_search_index RBAC predicates + redacted source

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 5.14
> **Phase**: 5
> **Type**: backend+security

---

## Goal

Per architecture §10 + §7.1: `session_search_index` (and the paired FTS5 virtual table) gains
`tenant_id` + `visibility_level` columns (V013 ALTER landed via story 5.2). Index INSERT now
reads `searchable_text` from `tenant_event_view`, NOT from raw `events.data`. Every query
builder enforces the compound predicate.

## Acceptance criteria

- [ ] `crate::search::session::query_builder` emits
      `WHERE tenant_id = :tenant AND visibility_level IN (:allowed) AND session_id IN (...)`.
- [ ] FTS triggers updated so `session_search_fts` stays in sync with the new columns.
- [ ] Index INSERT pulls from `tenant_event_view.searchable_text` (no double redaction).
- [ ] Failure path: if upstream projection skipped, index row is skipped too.
- [ ] Forged-tenant test: query with arbitrary tenant_id returns zero rows.

## Verification

```bash
cargo test -p seasoned-hand-core search::session::rbac_tests
```

## Refs

- requirements: F-5.11, F-5.13, NFR-5.6
- architecture: §10, §7.1
