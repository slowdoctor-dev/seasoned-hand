# Story 4.2 — Atomic slice: V011 + ADR-013 + ARCH v1.3 + taxonomy expansion + backfill/FTS semantics

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 4.1
> **Phase**: 4
> **Type**: backend+docs

---

## Goal

Land the Phase 4 atomic schema/spec slice in one PR: V011 migration, ADR-013, ARCHITECTURE.md
v1.2->v1.3 reconciliation, Skill taxonomy expansion (`curation_decision`), curator FTS surfaces,
and backfill semantics.

## Acceptance criteria

- [ ] `migrations/V011__phase4_curator.sql` lands with all tables/sketches from architecture §3.
- [ ] `ADR-013` documents V011 reconciliation and atomic-slice rule.
- [ ] `ARCHITECTURE.md` is updated to v1.3 with §2.5/§2.1 updates.
- [ ] `playbooks.source_project_id` + revision backfill semantics implemented.
- [ ] `curator_search_fts` maintenance triggers are defined.
- [ ] Event taxonomy documents `Skill.kind += curation_decision`.

## Non-goals

- Curator runtime logic.

---

## Implementation steps

1. Implement V011 migration + backfill + indexes + triggers.
2. Add ADR-013 + ARCH v1.3 updates in same slice.
3. Add migration idempotency tests.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.14, F-4.16, F-4.26, NFR-4.3
- architecture: §3, §4.5, §10
- debt closed: #77 (close), #102 (close)
