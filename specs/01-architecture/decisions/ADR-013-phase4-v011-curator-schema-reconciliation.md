# ADR-013: ARCHITECTURE.md v1.2 -> v1.3 (Phase 4 V011 curator schema reconciliation)

Status: Accepted
Date: 2026-05-18
Deciders: Project lead

## Context

Phase 4 story 4.2 is an atomic-slice foundation story. Per F-3.19 style reconciliation discipline
(used in ADR-012), this slice must land in one PR:

1. V011 migration for curator/revision surfaces
2. Event taxonomy expansion (`Skill.kind += curation_decision`)
3. ARCHITECTURE.md immutable-text reconciliation
4. Backfill semantics and new FTS5 maintenance triggers

Without same-slice reconciliation, the immutable architecture text would drift from shipped SQL and
runtime event semantics, repeating the exact failure mode ADR-012 was created to prevent.

## Decision

Bump `/specs/01-architecture/ARCHITECTURE.md` from **v1.2** to **v1.3** and reconcile:

1. **§2.5 schema surface** with V011 additions:
   - `playbooks` extension columns: `source_project_id`, `active_revision_id`,
     `archived_reason`, `archived_at`
   - new tables: `playbook_revisions`, `playbook_revision_outcomes`, `curator_decisions`,
     `curator_review_queue`, `sop_conflicts`, `knowledge_items`, `datasource_items`,
     `weekly_retrospectives`, `retrospective_citations`, `curator_search_index`
   - new FTS surface: `curator_search_fts` + maintenance triggers
     (`curator_search_index_ai`, `_ad`, `_au`)
2. **§2.1 event-kind taxonomy text** to include Phase 4 skill sub-kind:
   `curation_decision`.
3. **Backfill semantics** documented for V010 -> V011 data transition:
   - denormalize `source_project_id`
   - seed revision-1 rows from existing playbooks
   - seed revision outcomes from existing counters
   - rebuild `playbooks_fts` once post-backfill
4. **Atomic-slice rule reaffirmed**: migration + ADR + ARCH reconciliation ship together.

No stack/platform change is introduced by this ADR.

## Consequences

Positive:
- Immutable architecture text and runtime schema remain aligned.
- Phase 4 implementation stories can rely on §2.5 as authoritative, not stale.
- Event-stream consumers can treat `Skill.kind=curation_decision` as first-class.

Negative:
- §2.5 grows and now includes another phase-level schema extension layer.

Neutral:
- `tenant_id` forward-compat remains nullable in Phase 4 (Phase 5 tightens).

## Alternatives considered

### Alternative A: Land V011 SQL now, defer ARCH update
Rejected: violates AGENTS §8 reconciliation rule and repeats pre-ADR-012 drift pattern.

### Alternative B: Keep v1.2 and encode v1.3 details only in phase docs
Rejected: immutable ARCH would no longer be truthful for current runtime schema.

### Alternative C: Delay `curation_decision` taxonomy until runtime stories
Rejected: V011 atomic slice already introduces curator persistence; taxonomy expansion must be
co-documented now so replay/search tooling has a stable contract.

## References

- `/specs/phase-4/stories/story-4.2.md`
- `/specs/phase-4/requirements.md` (F-4.14, F-4.16, F-4.26, NFR-4.3)
- `/specs/phase-4/architecture.md` (§3, §4.5, §10)
- ADR-012 (Phase 3 reconciliation precedent)
- AGENTS.md §8 (same-PR spec/code reconciliation)
