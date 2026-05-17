# ADR-012: ARCHITECTURE.md v1.1 -> v1.2 (Phase 3 V010 learning schema reconciliation)

Status: Accepted
Date: 2026-05-18
Deciders: Project lead

## Context

Phase 3 story 3.2 must land an atomic schema/spec slice per F-3.19:

1. V010 migration for learning artifacts (`playbooks` extensions + `sops` + `glossary`)
2. FTS5 maintenance triggers for external-content virtual tables
3. Un-stubbed tool surfaces (`sop_read`, `playbook_search`, `glossary_lookup`)
4. Immutable architecture text reconciliation in the same PR slice

`ARCHITECTURE.md` v1.1 §2.5 captured the rich target shape but omitted V009 carry-forward
columns (`tenant_id`, `content_path`, `schema_version`, `source_task_id`) and did not
specify the external-content FTS5 maintenance triggers needed for correctness when
application code performs plain INSERT/UPDATE/DELETE on source rows.

Without same-slice reconciliation, this would reintroduce intentional drift windows
explicitly forbidden by AGENTS.md §8 and requirements F-3.19.

## Decision

Bump `/specs/01-architecture/ARCHITECTURE.md` from v1.1 to **v1.2** and reconcile §2.5
with shipped V010 reality:

1. Keep V009-compatible `playbooks` columns (`tenant_id`, `content_path`,
   `schema_version`, `source_task_id`) while adding Phase 3 columns
   (`trigger_keywords`, `content`, `success_count`, `failure_count`,
   `avg_duration_ms`, `avg_tool_calls`, `status`, `version`).
2. Keep `sops` and `glossary` as Phase 3 tables without `tenant_id` (single-operator
   boundary; Phase 5 adds tenant semantics).
3. Add denormalized `session_search_index` + `session_search_fts` to §2.5 as part of
   learning/search operability scope.
4. Explicitly include maintenance triggers for `playbooks_fts` and `session_search_fts`
   (`*_ai`, `*_ad`, `*_au`) to keep external-content FTS indexes consistent.

No stack/platform change; this is schema-text reconciliation under the existing SQLite+FTS5
persistence choice.

## Consequences

Positive:
- Story 3.2 satisfies F-3.19 atomicity: migration + tooling + immutable-spec reconciliation
  in one slice.
- Fresh sessions can rely on ARCH §2.5 matching V010 runtime behavior.
- FTS5 trigger behavior is now architecture-visible rather than hidden implementation detail.

Negative:
- §2.5 grows in detail and includes compatibility columns that are not directly used by
  every Phase 3 path.

Neutral:
- `content_path` remains reserved in Phase 3 (V009 compatibility) while `content` is the
  active playbook body; future curation can activate spill semantics without schema rollback.

## Alternatives considered

### Alternative A: Keep v1.1 and treat V010 as implementation detail
Rejected: violates F-3.19 and repeats drift that ADR-011 just resolved.

### Alternative B: Drop V009 compatibility columns in V010
Rejected: would require table-rewrite migration and lose low-risk forward compatibility.

### Alternative C: Defer FTS5 trigger specification to code comments/tests
Rejected: trigger correctness is architectural because index consistency is part of the
public operability contract (NFR-3.6).

## References

- `/specs/phase-3/stories/story-3.2.md`
- `/specs/phase-3/requirements.md` (F-3.5, F-3.10, F-3.16, F-3.19, F-3.21, NFR-3.6)
- `/specs/phase-3/architecture.md` (§3, §4, §10)
- ADR-011 (drift-consolidation precedent)
- AGENTS.md §8 (same-PR spec/code reconciliation)
