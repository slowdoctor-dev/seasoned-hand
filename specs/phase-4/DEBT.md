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

### Story 4.6 execution note

- `#89` **partially closed** by story 4.6 (ConflictDetector):
  - Added production semantic-adjudication prompt path (`LlmSemanticAdjudicator`) and structural
    prefilter + severity triage writes to `sop_conflicts`.
  - Remaining closure scope stays with story 4.7 retrospective refusal-hardening.

### Story 4.7 execution note

- `#75` **partially closed** by story 4.7 (RetrospectiveGenerator):
  - Added production retrospective generation with citation coverage validation and refusal on
    weak evidence (`coverage < 0.95`).
- `#89` **partially closed** by story 4.7:
  - Added retrospective-specific refusal prompt/validator path (`[[CIT:<kind>:<id>]]` policy).

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
- **Status**: closed (story 4.15)

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

## Story 4.10 close-out (2026-05-19)

- `#73` CLOSED by story 4.10:
  - production `SqliteKnowledgeDatasourceWriter` ships raw staging writes for `knowledge_items`
    and `datasource_items`, plus L2-gated promotion decisions (`knowledge_write` /
    `datasource_write`) in `curator_decisions`
  - per-artifact-class rollout flags are wired (`SH_CURATOR_L2_ENFORCE_KNOWLEDGE`,
    `SH_CURATOR_L2_ENFORCE_DATASOURCE`) and passed via Curator runtime dependency graph from
    `main.rs`
  - Curator cycle integrates writer execution in production path (`ProductionCuratorCycleExecutor`)
  - evidence:
    `crates/seasoned-hand-core/src/curator/mod.rs`,
    `crates/seasoned-hand-server/src/main.rs`,

## Story 4.13 close-out (2026-05-19)

- `#90` CLOSED by story 4.13:
  - archive recommendation/apply policy now has project-level thresholds in production runtime
    (`auto_archive_enabled`, `archive_recommend_min_confidence`, `archive_apply_min_confidence`)
    and is wired from server env config into `SqliteConsolidationEngine`
  - deterministic restore/unarchive path is implemented (`decision_type='restore'`) and keeps
    revision outcome counters intact during archive/restore roundtrip
  - archive provenance now persists confidence context in `playbooks.archived_reason`
  - evidence:
    `crates/seasoned-hand-core/src/curator/mod.rs`,
    `crates/seasoned-hand-server/src/main.rs`,
    `curator::tests::e2e_consolidation_archive_and_restore_roundtrip_preserves_outcome_counts`

- `#92` PARTIAL by story 4.13:
  - project-level static threshold policy from architecture §12.1 is now implemented and wired.
  - adaptive thresholding remains deferred to Phase 5 by original debt scope.
    `curator::tests::e2e_cycle_knowledge_datasource_emit_and_l2_promotion_paths`

## Story 4.11 close-out (2026-05-19)

- `#89` CLOSED by story 4.11:
  - curator cycle now emits explicit quarantine telemetry for LLM refusal and malformed-payload
    categories (`curator_decision_quarantined` with discriminants) instead of failing whole-cycle
  - confidence composition now enforces deterministic floor + bounded LLM contribution cap (+0.45)
    for adversarial resistance per architecture §9.1
  - evidence:
    `crates/seasoned-hand-core/src/curator/mod.rs`,
    `curator::tests::run_once_emits_quarantine_events_for_all_failure_categories`,
    `curator::tests::adversarial_confidence_bounds_enforce_deterministic_floor`
- `#100` CLOSED by story 4.11:
  - runtime now distinguishes batch-scope OOM containment from process-level failure behavior by
    proactively splitting oversized candidate batches once and quarantining that decision unit
    (`failure_category='out_of_memory'`) while continuing cycle work
  - SQLite BUSY contention now has bounded backoff/retry (50/100/200/400ms) before quarantine,
    avoiding all-or-nothing cycle aborts
  - evidence:
    `crates/seasoned-hand-core/src/curator/mod.rs` (`apply_with_busy_backoff`,
    batch split guard in `ProductionCuratorCycleExecutor::execute`)

## Story 4.12 close-out (2026-05-19)

- `#88` CLOSED by story 4.12:
  - Curator emits taxonomy-aligned telemetry events:
    - `Misc.kind=curator_cycle_started`
    - `Misc.kind=curator_cycle_completed`
    - `Misc.kind=curator_decision_quarantined`
    - `Misc.kind=curator_budget_circuit_open`
    - `Misc.kind=curator_retrospective_refused`
  - Curator now emits `Skill.kind=curation_decision` events from cycle decision ledger rows with
    canonical payload fields (`decision_type`, `subject_id`, `confidence`, `review_state`)
  - Session-search indexing is regression-tested for `curation_decision` discoverability
  - evidence:
    `crates/seasoned-hand-core/src/curator/mod.rs`,
    `curator::tests::emits_curation_decision_skill_and_curator_misc_taxonomy_events`

## Story 4.14 close-out (2026-05-19)

- `#91` CLOSED by story 4.14:
  - `SH_CURATOR_*` env values now use strict parser semantics with explicit startup errors for
    invalid booleans/numbers instead of permissive fallback coercion
  - strict boolean parsing also applied to L2 enforcement flags
    (`SH_CURATOR_L2_ENFORCE_KNOWLEDGE`, `SH_CURATOR_L2_ENFORCE_DATASOURCE`)
  - evidence:
    `crates/seasoned-hand-server/src/main.rs` (`load_curator_config_from_lookup`,
    `env_bool_or_default`, strict parser tests in `src/main.rs`)

- `#98` CLOSED by story 4.14:
  - configuration surface is explicitly split into
    `embedding_budget_soft_cap_pct` and `embedding_budget_hard_breaker_pct` with strict range and
    monotonicity checks (`hard >= soft`)
  - zero-baseline fallback behavior is regression-tested using absolute monthly budget cap
  - evidence:
    `crates/seasoned-hand-server/src/main.rs`,
    `crates/seasoned-hand-core/src/curator/mod.rs`,
    `tests::embedding_budget_zero_baseline_fallback_is_absolute_cap`,
    `curator::tests::embedding_budget_uses_monthly_token_fallback_when_total_tokens_zero`

## Story 4.15 close-out (2026-05-19)

- `#95` CLOSED by story 4.15:
  - project-scope isolation now has explicit fail-closed regression coverage for cross-project
    consolidation decisions and conflict writes, plus review-queue project-scoped query behavior
  - cross-project revision references are rejected before curator decision insertion
  - evidence:
    `crates/seasoned-hand-core/src/curator/mod.rs`
    (`validate_decision_scope`, `consolidation_apply_rejects_cross_project_revision_scope`,
    `conflict_detector_rejects_cross_project_pairs_without_writes`,
    `review_queue_transitions_are_scoped_to_target_queue_project_rows`)
