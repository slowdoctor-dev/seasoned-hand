# Story 3.2 — Atomic slice: V010 + ADR-012 + ARCH v1.2 + FTS5 triggers + tool un-stubs

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 3.1
> **Phase**: 3
> **Type**: backend+docs

---

## Goal

Land the Phase 3 atomic schema/spec slice required by F-3.19 in one PR: V010 migration,
ADR-012 + ARCHITECTURE.md §2.5 v1.1→v1.2 reconciliation, FTS5 maintenance triggers,
and real implementations for `sop_read`, `playbook_search`, `glossary_lookup`.

## Acceptance criteria

- [ ] `V010__phase3_learning_artifacts.sql` lands with playbook extensions, `sops`,
      `glossary`, `playbooks_fts`, `session_search_fts`, and maintenance triggers.
- [ ] New ADR-012 documents the reconciliation rationale and atomic-slice rule.
- [ ] `ARCHITECTURE.md` is bumped to v1.2 with reconciled §2.5 schema text.
- [ ] `sop_read`, `playbook_search`, `glossary_lookup` stubs are removed and return real rows.
- [ ] Existing behavior outside learning surfaces remains unchanged.
- [ ] `bash scripts/spec-check.sh` passes.

## Non-goals

- Extraction orchestration and safety logic.
- Benchmark/regression test suite.

---

## Implementation steps

1. Implement V010 migration, indexes, and FTS5 triggers.
2. Add ADR-012 and ARCH v1.2 updates in the same slice.
3. Wire tool implementations to V010 schema and project-scope constraints.
4. Add/adjust migration and tool unit tests.

---

## Verification

```bash
cargo test -p seasoned-hand-core migration::v010
cargo test -p seasoned-hand-core tools::builtin::learning_tools
bash scripts/spec-check.sh
```

---

## Refs

- requirements: F-3.5, F-3.10, F-3.16, F-3.19, F-3.21, NFR-3.6
- architecture: §3, §4, §10
- debt closed: #78 (H), Phase 2 DEBT #6
