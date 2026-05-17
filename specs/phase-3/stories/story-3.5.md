# Story 3.5 — Matcher core (gate identity + production FTS5 + deterministic ranking)

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 3.2
> **Phase**: 3
> **Type**: backend

---

## Goal

Implement both matchers with shared normalization, project-scoped filtering, FTS5 scoring,
and deterministic tie-breaking for top candidate ordering.

## Acceptance criteria

- [ ] Shared normalizer is NFD + lowercase + whitespace-collapse + trim.
- [ ] Gate mode matches by fixture_id + normalized_brief strict identity.
- [ ] Production mode queries FTS5 prefix over keywords/title/content.
- [ ] Matching is project-scoped through `source_task.project_id` equality.
- [ ] Ranking is deterministic with pinned secondary/tertiary/final keys.
- [ ] Matcher mode is runtime configurable and emits `Skill{kind:"match"}` on hit.

## Non-goals

- Prompt injection and byte-cap truncation.

---

## Implementation steps

1. Add shared normalization utility and tests.
2. Implement gate matcher and production FTS matcher.
3. Apply deterministic ranking function and skill-event emission.

---

## Verification

```bash
cargo test -p seasoned-hand-core matcher::normalization
cargo test -p seasoned-hand-core matcher::gate_identity
cargo test -p seasoned-hand-core matcher::fts_ranking_determinism
```

---

## Refs

- requirements: F-3.4, F-3.5, F-3.12, NFR-3.2
- architecture: §2, §3, §11
