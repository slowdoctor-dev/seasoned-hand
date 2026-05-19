# Phase 4 — Technical Debt Ledger

Date: 2026-05-18  
Owner: BMAD Analyst seed

This ledger starts with inherited debt from Phase 3 that Phase 4 is expected to close or
explicitly partially close with evidence.

## Inherited pay-down targets from Phase 3

- `#72` Embedding rerank over FTS5 candidates was deferred; Phase 4 Curator consolidation must
  close this to reduce semantic duplicate misses.
- `#73` Knowledge/Datasource writers and L2 cross-source enforcement were deferred; Phase 4 must
  land production write paths and enforcement rollout.
- `#75` Summarizer hardening deferred; Phase 4 retrospectives must add grounded citation/refusal
  behavior.
- `#76` FTS5 weighting retune deferred; Phase 4 should at minimum partially close with hybrid
  candidate + rerank tuning evidence.
- `#77` `source_project_id` denormalization deferred; Phase 4 schema should close this for faster
  project-scoped curation queries.
- `#87` Warm benchmark with full learning loop deferred; Phase 4 close-out requires this gate.
- `#88` Event-kind taxonomy/documentation drift deferred; Phase 4 curator events must reconcile.
- `#89` LLM refusal prompt specificity deferred; Phase 4 conflict/retrospective prompts should
  close or partially close with explicit validation.
- `#90` Dedup guard deferred; Phase 4 auto-merge/archive policy must close with false-positive
  bounds.
- `#91` Config/env parsing strictness deferred; Phase 4 curator flags should close or partially
  close with strict parsing semantics.

## Expected closure policy for inherited items

- Phase 4 close-out is expected to close: `#72`, `#73`, `#77`, `#87`, `#88`, `#89`, `#90`.
- Phase 4 may partially close: `#75`, `#76`, `#91` if broader global harmonization is deferred.
- Any non-closure must include rationale + successor phase owner in this file.

## Categories quick-reference

Phase 4 uses a finer-grained 4-tier scheme. Phase 0-3 used H/M/L. Cross-phase
audits should use this mapping when comparing:

| Phase 4 | Phase 0-3 | Meaning | Default handling |
|---|---|---|---|
| H0 | H | Release-blocking correctness/safety issue | Must fix in current slice |
| H1 | H | High risk, user-visible correctness/regression | Fix this phase unless explicit ADR |
| H2 | M | Medium risk, bounded workaround exists | Plan in active phase with owner |
| H3 | L | Low risk or optimization | Schedule when capacity allows |

The mapping is approximate; Phase 4's H0/H1 split distinguishes "blocking" (H0)
from "high-risk-but-not-blocking" (H1) — both map to Phase 3's single "H".

## Seed (TBD)

Architect and PM passes will append concrete entries in this format:

```md
### #NN Title (H1)
- **Opened**: YYYY-MM-DD by <role>
- **Context**: ...
- **Impact**: ...
- **Plan**: ...
- **Owner**: ...
- **Target phase/story**: ...
- **Status**: open
```

## Analyst note

This file is intentionally lean at kickoff. The key function is inheritance clarity and closure
accountability, not speculative implementation details.

## Architect seed additions (from OPEN_QUESTIONS resolutions, 2026-05-18)

### #92 Adaptive auto-archive thresholds (H3)
- **Opened**: 2026-05-18 by Architect
- **Context**: Q1 chose project-level static thresholds (option B) over adaptive distributional mode.
- **Impact**: archive precision may lag as corpus distribution shifts.
- **Plan**: evaluate adaptive thresholds after Phase 4 telemetry stabilizes.
- **Owner**: Phase 5 Architect
- **Target phase/story**: Phase 5 curation-tuning slice
- **Status**: open

### #93 Optional fork-promotion governance mode (H3)
- **Opened**: 2026-05-18 by Architect
- **Context**: Q3/Q10 chose mandatory revision chain over fork-promotion default.
- **Impact**: highly regulated teams may want stricter manual promotion semantics.
- **Plan**: add optional policy mode (fork + explicit promote) behind feature flag.
- **Owner**: PM/Architect (Phase 5)
- **Target phase/story**: Phase 5 policy variants
- **Status**: open

### #94 Retrospective tiered model-by-size policy (H3)
- **Opened**: 2026-05-18 by Architect
- **Context**: Q7 chose dedicated summarizer profile, deferred size-tiered routing.
- **Impact**: potential cost/quality optimization left on table for extreme report sizes.
- **Plan**: evaluate size-aware profile switching once baseline quality metrics are stable.
- **Owner**: Phase 5 Architect
- **Target phase/story**: Phase 5 retrospective optimization
- **Status**: open

### #95 Cross-project read-only analytics pilot (H3)
- **Opened**: 2026-05-18 by Architect
- **Context**: Q18 chose strict project isolation in Phase 4.
- **Impact**: no early network-effect insight across projects.
- **Plan**: design read-only, opt-in analytics path with strong isolation controls.
- **Owner**: Phase 5 Architect
- **Target phase/story**: Phase 5 multi-project analytics
- **Status**: open

### #96 Curator rationale schema evolution tooling (H3)
- **Opened**: 2026-05-18 by Architect
- **Context**: Q17 chose structured rationale object; schema version migration tooling deferred.
- **Impact**: future rationale shape changes could create migration friction.
- **Plan**: add rationale schema registry + migration validator.
- **Owner**: Phase 5 Architect
- **Target phase/story**: Phase 5 data evolution hardening
- **Status**: open

## Architect REVIEW iter-1 (Claude) — DEBT seeds for L-severity findings

### #97 Per-crate justification for new dependencies (H3)
- **Opened**: 2026-05-18 by Claude review iter-1
- **Context**: §5 lists `cron`/`ordered-float`/`lru` without per-crate concrete justification.
  `cron` may be over-engineered for simple weekly cadence (a Tokio interval timer + day-of-week
  check suffices); `ordered-float` may be avoidable if score f32 comparisons happen at decision
  sites, not as map keys.
- **Impact**: 3 new dep adoptions trigger 3 separate ARCH §1 addendum updates + audit surface.
- **Plan**: Architect/PM stories drop or justify each before adoption. Default to standard
  library where pragmatic.
- **Owner**: Architect (Phase 4 implementation slice)
- **Target phase/story**: each crate-adoption story
- **Status**: open

### #98 Split embedding budget cap field (H3)
- **Opened**: 2026-05-18 by Claude review iter-1
- **Context**: §4.1 `CuratorConfig.embedding_budget_percent_cap: f32` is one field but NFR-4.6
  defines two thresholds: soft 8% + hard 12%.
- **Impact**: ambiguous which cap the single field represents; implementation could ship one or
  the other but not both.
- **Plan**: Split to `embedding_budget_soft_cap_pct: f32` + `embedding_budget_hard_breaker_pct: f32`.
- **Owner**: PM (config story)
- **Target phase/story**: Phase 4 CuratorConfig story
- **Status**: open

### #99 §7 cycle budget amortization arithmetic (H3)
- **Opened**: 2026-05-18 by Claude review iter-1
- **Context**: §7 subcomponent budgets sum to 4800ms but total cycle bound is 4000ms. The gap
  is explained by retrospective being "weekly amortized" but the amortization formula isn't
  written.
- **Plan**: pin formula explicitly — per-cycle = 400+1800+700+700 = 3600ms (excluding
  retrospective); retrospective +1200ms once weekly. Total weekly = 4 × 3600 + 1200 = 15600ms.
- **Owner**: Architect (next iter-2 or PM stories)
- **Target phase/story**: Phase 4 §7 hardening or first benchmark story
- **Status**: open

### #100 F-4.22 OOM handling: process vs batch (H3)
- **Opened**: 2026-05-18 by Claude review iter-1
- **Context**: §8 category 5 "Out-of-memory: split candidate batch size by half and retry once"
  treats OOM as controlled-allocation failure. Rust process-level OOM = allocator abort, no
  retry possible at userspace.
- **Impact**: implementation may try the "retry once" pattern on process-OOM and crash differently
  than expected.
- **Plan**: distinguish (a) batch-OOM (predicted by candidate size; controlled retry with smaller
  batch) from (b) process-OOM (allocator abort; supervisor restarts CuratorWorker; emit
  `curator_decision_quarantined` post-restart).
- **Owner**: PM (CuratorWorker failure-handling story)
- **Target phase/story**: Phase 4 CuratorWorker failure story
- **Status**: open

### #101 phase4_warm_full_loop_benchmark concrete setup (H3)
- **Opened**: 2026-05-18 by Claude review iter-1
- **Context**: §11.3 mentions `phase4_warm_full_loop_benchmark` for F-4.21 but in one line.
  Phase 3 phase3_warm_benchmark had a concrete setup: seed → cold → matcher → injection → warm
  → assert. Phase 4 should mirror.
- **Plan**: §11.3 (or new §11.5) describes: 200-artifact stub corpus → cold curator cycle →
  apply auto-merge/auto-archive decisions → warm curator cycle measures improvement vs cold.
  Specify which metric (precision@3? stale ratio?) is the gate.
- **Owner**: PM (benchmark fixture story)
- **Target phase/story**: Phase 4 F-4.21 fixture story
- **Status**: open

### #102 playbook_revisions FOREIGN KEY for parent_revision_id (H3)
- **Opened**: 2026-05-18 by Claude review iter-1
- **Context**: §3.2 `playbook_revisions.parent_revision_id TEXT` (nullable) has no FOREIGN KEY
  reference. Graph integrity depends on writer discipline alone.
- **Plan**: add `FOREIGN KEY(parent_revision_id) REFERENCES playbook_revisions(id) ON DELETE SET NULL`
  to schema. Catches dangling parents at write time.
- **Owner**: Architect (next iter) or PM (V011 migration story)
- **Target phase/story**: Phase 4 V011 migration story
- **Status**: open

## Story 4.2 close-out (2026-05-18)

- `#77` CLOSED by story 4.2:
  - V011 adds `playbooks.source_project_id` + `idx_playbooks_project_status`
  - V011 backfill sets `source_project_id` from `tasks.project_id`
  - evidence: `migrations/V011__phase4_curator.sql`, `db::tests::migration_v011_backfill_from_v010_rows`
- `#102` CLOSED by story 4.2:
  - V011 defines `FOREIGN KEY(parent_revision_id) REFERENCES playbook_revisions(id) ON DELETE SET NULL`
  - evidence: `migrations/V011__phase4_curator.sql`, `story-4.2.md` acceptance + refs

## Story 4.4 close-out (2026-05-19)

- `#72` CLOSED by story 4.4:
  - production `SqliteCandidateBuilder` + `ProductionEmbeddingReranker` wired into Curator runtime
  - embedding endpoint contract implemented (`POST /v1/embeddings`) with configured model and
    blend/fallback formulas from architecture §4.2
  - evidence: `crates/seasoned-hand-core/src/curator/mod.rs`,
    `crates/seasoned-hand-core/src/llm/{mod.rs,types.rs}`,
    `crates/seasoned-hand-server/src/main.rs`,
    `curator::tests::embedding_enabled_and_fallback_paths_are_exercised`
- `#99` CLOSED by story 4.4:
  - cycle runtime now emits measured per-cycle `elapsed_ms` from production candidate/rerank path,
    establishing concrete cycle-time accounting for budget validation (the §7 amortization check
    now has runtime evidence instead of prose-only estimates)
  - evidence: `ProductionCuratorCycleExecutor::execute` elapsed accounting in
    `crates/seasoned-hand-core/src/curator/mod.rs` and cycle telemetry assertions in
    `curator::tests::run_once_emits_cycle_start_and_complete_events`

## Story 4.5 close-out (2026-05-19)

- `#90` PARTIAL by story 4.5:
  - production `SqliteConsolidationEngine` now applies merge/keep/quarantine/archive-recommend
    policy with confidence-band review gating and deterministic review sampling
  - revision-chain writes are transactional (`playbook_revisions` insert + supersede + active
    revision pointer update) for merge paths
  - low-confidence and high-impact decisions are queued into `curator_review_queue`
  - evidence: `crates/seasoned-hand-core/src/curator/mod.rs` (`SqliteConsolidationEngine`,
    `review_required`, `apply_merge`), and
    `curator::tests::e2e_cycle_covers_merge_and_keep_branches_with_stubbed_rerank`
  - residual to close in later stories: full false-positive audit harness and archive
    reversibility acceptance gates (stories 4.13 + 4.16)
