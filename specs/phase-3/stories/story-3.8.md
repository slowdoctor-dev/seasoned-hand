# Story 3.8 — Session search index ingestion and internal query API

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 3.2
> **Phase**: 3
> **Type**: backend

---

## Goal

Ship denormalized session-search indexing for all 8 event types with internal query API,
without requiring replay scans.

## Acceptance criteria

- [ ] Event append path writes synchronized `session_search_index` rows.
- [ ] FTS5 search is queryable across all 8 EventType values.
- [ ] Phase 3 active writers index Message/Action/Observation/Plan/Skill/Misc.
- [ ] Reserved Knowledge/Datasource variants remain schema-covered and queryable.
- [ ] Query API supports filters (session/type/time) and raw hit snippets.

## Non-goals

- CLI surface and LLM summarization UX.

---

## Implementation steps

1. Implement event-to-searchable-text denormalization.
2. Wire transactional ingestion path.
3. Add internal query API and integration tests.

---

## Verification

```bash
cargo test -p seasoned-hand-core session_search::index_ingestion
cargo test -p seasoned-hand-core session_search::all_event_types_queryable
```

---

## Refs

- requirements: F-3.16, NFR-3.6
- architecture: §3, §4
- debt context: Phase 2 DEBT #61 — story 3.7 closes the `Skill` writer portion;
  `Knowledge` / `Datasource` writers remain Phase 4+. This story makes the search-index
  schema ready for those future writers (no migration required at Phase 4 emit time),
  but does NOT itself close any portion of DEBT #61.
