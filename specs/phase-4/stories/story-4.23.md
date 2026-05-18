# Story 4.23 — Curator telemetry retention + compaction (NFR-4.4 close)

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 4.2, 4.12
> **Phase**: 4
> **Type**: backend+test
> **Origin**: PM REVIEW iter-1 (Claude, post-`1e11a0f`) — NFR-4.4 had zero story coverage

---

## Goal

Implement the runtime that keeps Curator telemetry within NFR-4.4 bounds. Phase 4
emits substantial telemetry (`curator_*` Misc events, `Skill{kind:"curation_decision"}`
events, `curator_decisions` ledger, `weekly_retrospectives` + `retrospective_citations`,
`curator_search_index`); without explicit retention/compaction, the per-project SQLite
DB grows unbounded and the 300 MB/month/project cap fails. This story owns the
retention scheduler, tiered raw→summarized compaction (per OPEN_QUESTIONS §12.12
option B), and storage cap enforcement.

Without this story, Phase 4 ships an emit pipeline with no compaction tail — the
NFR-4.4 acceptance bound becomes paper-only. Story 4.22 close-out cannot honestly
mark NFR-4.4 satisfied.

## Acceptance criteria

- [ ] A `CuratorRetentionJob` (or similarly-named) production component lives under
      `crates/seasoned-hand-core/src/curator/retention.rs` (or under a `curator/`
      module structure consistent with story 4.3's CuratorWorker layout).
- [ ] The job runs on a configurable interval (default: daily at 02:00 UTC project-local,
      configurable via `SH_CURATOR_RETENTION_CRON`) AND on storage-cap-exceeded triggers
      emitted by other Curator components.
- [ ] **Hot retention window** = 90 days as pinned by NFR-4.4. Data older than 90 days
      transitions from "raw" to "summarized" tier:
      - `curator_decisions` rows ≥90 days old: collapsed into per-week
        `curator_decisions_summary` records (count, decision_type histogram, mean
        confidence, project_id) + the raw rows are deleted in the same transaction.
      - `weekly_retrospectives` retained as-is (already summarized form); only
        `retrospective_citations` rows ≥90 days old get pruned (the retrospective
        narrative itself remains).
      - `curator_search_index` rows ≥90 days old get pruned (with FTS5 trigger handling
        the corresponding `curator_search_fts` delete).
- [ ] **Storage cap enforcement**: when project SQLite size exceeds 300 MB (NFR-4.4
      bound), the job emits `Misc{kind:"curator_storage_cap_warning", project_id,
      current_bytes, cap_bytes}` and accelerates compaction (drops to a 60-day hot window
      until size returns under cap).
- [ ] **Atomicity** per NFR-4.3: each compaction batch commits in one transaction; no
      partial state externally visible.
- [ ] Idempotent: re-running compaction on already-summarized data is a no-op.
- [ ] Emits `Misc{kind:"curator_retention_cycle_completed", project_id, raw_pruned,
      summarized_written, elapsed_ms}` telemetry per cycle for observability.

## Non-goals

- Per-project retention class customization beyond the daily cron + cap-triggered runs
  (OPEN_QUESTIONS §12.12 deferred per-project retention classes; DEBT seed if needed).
- Cross-project compaction (project isolation per F-4.24 holds).
- Backup / restore tooling for pruned data (out of Phase 4 scope; operators can dump
  SQLite before compaction if desired).

---

## Implementation steps

1. Add `crates/seasoned-hand-core/src/curator/retention.rs` with `CuratorRetentionJob`
   struct + Tokio task spawned by CuratorWorker boot (or as a sibling task).
2. Add `curator_decisions_summary` table to V011 (extend story 4.2's migration) OR
   emit summary records inside `curator_decisions` with a `decision_type='summary'`
   discriminant. PM's call — both work; the summary table is cleaner.
3. Implement the daily cron tick + cap-exceeded trigger handling.
4. Implement the per-table pruning queries with explicit transactional commit.
5. Wire the storage-cap-warning event emit + accelerated compaction path.
6. Wire `SH_CURATOR_RETENTION_CRON` env parsing via story 4.14's strict-parser
   (default daily 02:00 UTC).
7. Integration test: seed >300 MB of synthetic decisions, trigger compaction, assert
   size drops + summary rows appear + raw rows pruned + idempotent re-run is no-op.
8. Unit tests: cron-parse correctness, transaction rollback on injected failure,
   90-day window boundary correctness.
9. Update `weekly_retrospectives.citation_coverage` recomputation if citation rows
   get pruned (citations beyond window count toward original `citation_coverage`
   but cannot be inspected post-prune — document this in retrospective metadata).

---

## Verification

```bash
# Full AGENTS.md §6 gate list (Phase 3 REVIEW iter-1 F7 lesson)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
pnpm typecheck
pnpm test
bash scripts/spec-check.sh

# Story-4.23-specific evidence
cargo test -p seasoned-hand-core curator::retention::compaction_window_boundary
cargo test -p seasoned-hand-core curator::retention::storage_cap_trigger
cargo test -p seasoned-hand-core curator::retention::idempotent_rerun
```

---

## Refs

- requirements: NFR-4.4 (full close), NFR-4.3 (atomicity reuse)
- architecture: §3.2 (curator_decisions, retrospective_citations, curator_search_index), §12.12 (tiered retention resolution)
- debt closed: closes NFR-4.4 coverage gap (PM REVIEW iter-1 finding)

## Notes

This story was added by PM REVIEW iter-1 (Claude) after the initial 22-story breakdown
(`1e11a0f`) missed NFR-4.4 coverage. Mirror of Phase 3's post-hoc story 3.17 pattern —
when REVIEW catches a structural gap, the fix is a dedicated story, not extending an
unrelated story to cover the gap. Phase 4 PM-iter-1 caught this BEFORE story execution
started (whereas Phase 3 caught it AFTER 16 stories had already shipped), so the cost
of recovery is minimal.
