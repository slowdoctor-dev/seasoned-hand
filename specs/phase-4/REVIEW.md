# Phase 4 — Cross-phase Hardening Review (Claude iter-1)

> Date: 2026-05-18
> Reviewer: Claude (iter-1 of alternating Claude/Codex hardening)
> Scope: Codex Analyst output at `628893a` (requirements.md, INPUTS.md,
> OPEN_QUESTIONS.md, DEBT.md, architecture.md placeholder)
> Pattern mirrors `/specs/phase-3/REVIEW.md` iter-1..iter-5 sequence.

## Findings summary

| # | Severity | Category | Title |
|---|---|---|---|
| P4-IT1-F1 | **M** | cross-phase consistency | DEBT severity scheme drift (H0/H1/H2/H3 vs Phase 3 H/M/L) |
| P4-IT1-F2 | **M** | scope vs design | F-4.7 "Architect resolves" defers a scope decision (revision identity affects matcher/counter/injector contracts); no cross-ref to OPEN_QUESTIONS #10 |
| P4-IT1-F3 | L | NFR rigor | NFR-4.4/4.6/4.7 had under-specified baselines (load profile, zero-baseline projects, sample-set floor) |
| P4-IT1-F4 | L | acceptance rigor | Acceptance #1 "200 verified artifacts in CI replay" — no wall-clock budget |
| P4-IT1-F5 | L | cross-ref hygiene | F-4.3 revision granularity doesn't cross-ref F-4.7 / OPEN_QUESTIONS #10 |
| P4-IT1-F6 | **M** | failure taxonomy | F-4.22 failure containment narrow (exceptions/refusals/payloads only — missed timeout, OOM, DB lock, slot-router failure) |
| P4-IT1-F7 | L | baseline-floor pin | F-4.10 SOP conflict algorithm not pinned at "minimum baseline" level (Phase 3 F-3.13 precedent) |
| P4-IT1-F8 | L | cross-ref hygiene | F-4.8 + F-4.10 + F-4.7 should cross-link OPEN_QUESTIONS #11/9/10 respectively |

All 8 fixed inline in this commit (no DEBT seeded — these were spec-tightening,
not deferred work).

---

## P4-IT1-F1 (M, FIXED) — DEBT severity scheme drift

**Evidence** — Codex's DEBT.md introduced `H0/H1/H2/H3` for Phase 4. Phase 0/1/2/3
used `H/M/L`. Cross-phase debt audits (e.g. "list all H-severity items across
phases") would silently miss Phase 4's H1/H2/H3 entries or double-count.

**Fix applied** — kept the finer 4-tier scheme (it's a real improvement — H0
"blocking" vs H1 "high-risk" is a useful distinction) but added an explicit
mapping table in DEBT.md:

| Phase 4 | Phase 0-3 |
|---|---|
| H0 | H |
| H1 | H |
| H2 | M |
| H3 | L |

Phase 4 audits can use either notation; cross-phase scripts treat them
equivalent via the mapping.

---

## P4-IT1-F2 (M, FIXED) — F-4.7 defers scope to Architect without cross-ref

**Evidence** — F-4.7 said "Skill self-improvement must produce revisioned updates
or versioned forks (Architect resolves)". The choice between revisioned-in-place
(same `playbook_id`, `version++`) vs versioned-fork (new `playbook_id`,
`superseded_by` link) is a SCOPE decision: it changes what F-4.3 success-rate
metrics key on, what F-4.6 consolidation merges into, what F-4.20 recommendations
reference. Architect handles HOW; Analyst should at least flag the cross-cutting
consequence.

**Fix applied** — F-4.7 now references OPEN_QUESTIONS #10 explicitly, calls out
that F-4.3 / F-4.6 / F-4.20 must use the same revision-identity choice. The
choice itself stays with Architect, but the consequence chain is documented.

---

## P4-IT1-F3 (L, FIXED) — NFR baselines tightened

**Evidence**

- NFR-4.4 said "300MB/month/project under expected load profile" — load profile
  undefined.
- NFR-4.6 said "<= 8% of total monthly token spend" — undefined behavior for
  zero-spend / startup projects.
- NFR-4.7 said "audited sample set defined by Architect/PM test harness" —
  minimum sample size unspecified, but ≤2% false-positive bound needs N≥50 per
  decision class to distinguish from 5% at 95% CI.

**Fix applied**

- NFR-4.4: pinned "expected load profile" = 100 verified artifacts/week + 4
  weekly retrospectives + 28 daily cycles. Architect may adjust constants but
  must preserve the per-project storage bound.
- NFR-4.6: added zero-baseline fallback = 50_000 embedding tokens/month until
  baseline establishes (≥7 days with >1k tokens/day).
- NFR-4.7: pinned minimum sample size = N≥100 per decision class across ≥3
  representative corpus shapes. Statistical justification included in body.

---

## P4-IT1-F4 (L, FIXED) — Acceptance #1 wall-clock budget

**Evidence** — Acceptance criterion #1 required "200 verified task-complete
artifacts in CI replay" but didn't pin wall-clock budget. Each artifact could
encode 50+ tool calls; naive replay = hours of CI.

**Fix applied** — pinned ≤45 min wall-clock on baseline runner; amortizes via
stub LLM + pre-rendered transcripts. Architect may relax with rationale if
infeasible.

---

## P4-IT1-F5 (L, FIXED) — F-4.3 cross-ref to F-4.7

**Evidence** — F-4.3 spoke of "revision granularity" without referencing F-4.7
(which actually defines what a revision IS) or OPEN_QUESTIONS #10 (which
captures the unresolved options).

**Fix applied** — F-4.3 body now cross-refs F-4.7 + OPEN_QUESTIONS #10, and
explicitly notes that F-4.3 / F-4.6 / F-4.20 must use the same revision-identity
choice for consistent learned-graph state.

---

## P4-IT1-F6 (M, FIXED) — F-4.22 failure taxonomy too narrow

**Evidence** — F-4.22 listed "exceptions, refusals, or bad payloads" as the
containment-triggering failure modes. The Phase 3 extraction handler experience
showed timeout, slot-router failure, and parse-output errors as DISTINCT event
classes deserving first-class treatment. Phase 4 Curator has additional failure
surfaces: DB lock contention (BUSY), OOM under heavy embedding batch, slot
misconfiguration.

**Fix applied** — F-4.22 now enumerates 7 failure categories: panic/error, LLM
refusal, malformed payload, timeout (NFR-4.1 bound exceeded), OOM, DB lock
contention after retry budget, slot-router resolution failure. Each quarantine
emits a Misc telemetry event with `failure_category` discriminant for
operational triage.

---

## P4-IT1-F7 (L, FIXED) — F-4.10 SOP conflict baseline algorithm not pinned

**Evidence** — F-4.10 described the goal (detect contradictory guidance) but
deferred algorithm entirely to OPEN_QUESTIONS #9. Phase 3 F-3.13 set the
precedent of pinning a MINIMUM BASELINE in the requirement body (specific
deterministic detection categories) and letting Architect curate extensions.
F-4.10 should mirror.

**Fix applied** — F-4.10 now pins minimum baseline = structural-step diff +
LLM-judged semantic contradiction (both signals must agree above lowest severity
tier). Architect may extend per OPEN_QUESTIONS #9.

---

## P4-IT1-F8 (L, FIXED) — Cross-ref hygiene to OPEN_QUESTIONS

**Evidence** — F-4.8 (auto-archive), F-4.10 (SOP conflict), F-4.7 (revisioning)
each had a corresponding OPEN_QUESTIONS entry (#11, #9, #10) but the F-body
didn't link to it. A reader of requirements.md alone wouldn't know the resolution
trail.

**Fix applied** — all three Fs now cross-ref their respective OPEN_QUESTIONS
sections inline.

---

## Iter-1 conclusion

All 8 findings fixed inline; no DEBT seeded (these were spec-tightening, not
deferred work). The Analyst output is now appropriately rigorous to hand to the
BMAD Architect persona:
- 26 F-numbers, all with "why this matters" rationale and (where applicable)
  cross-refs to OPEN_QUESTIONS
- 8 NFRs, all with measurable bounds + edge-case behavior
- 20 OPEN_QUESTIONS with neutral A/B/C/D tradeoff matrices
- DEBT.md scaffolds the carry-forward with cross-phase severity mapping
- Acceptance criteria are operationally defined with concrete budgets

Recommend dispatching Codex Architect next. If the user wants iter-2 (Codex
review of this iter-1) for symmetry with the Phase 3 5-iteration cadence,
that's fine but probably not load-bearing — the Analyst output is now in the
"editorial nits only" zone.

## REVIEW iter-2 (Codex, 2026-05-18)

### A) Grade Claude iter-1 findings (F1-F8)

Verdict: **8/8 agree**, **0 disagree**.

- `P4-IT1-F1` (M): agree on root cause and fix. The H0/H1/H2/H3 -> H/M/L mapping keeps
  Phase 4 fidelity while preserving cross-phase comparability; this is better than flattening back
  to H/M/L only.
- `P4-IT1-F2` (M): agree. F-4.7 had cross-cutting scope implications and now correctly links the
  dependency chain (F-4.3/F-4.6/F-4.20 + OQ #10).
- `P4-IT1-F3` (L): agree. Added NFR baselines are reasonable defaults. They are explicit enough for
  Architect retuning without over-constraining implementation.
- `P4-IT1-F4` (L): agree. Adding CI wall-clock bound closes a real acceptance ambiguity.
- `P4-IT1-F5` (L): agree. Cross-reference tightening was needed for traceability.
- `P4-IT1-F6` (M): agree. Expanded failure taxonomy closes a meaningful robustness gap.
- `P4-IT1-F7` (L): agree. Baseline algorithm pin is appropriate at Analyst level.
- `P4-IT1-F8` (L): agree. OPEN_QUESTIONS linkage now meets Phase 3 traceability precedent.

NFR baseline commentary: the iter-1 constants are acceptable **initial defaults**, not final tuning.
They are framed as adjustable by Architect with rationale, which is the right balance at Analyst
stage.

### B) Independent re-audit

#### OPEN_QUESTIONS #4-#20

Checked all sections (#4 through #20): each has context, 3-option (or more where applicable)
tradeoff framing, neutral pros/cons, and explicit suggested owner/next-step. No structural gaps.

#### INPUTS.md quality

- Code-surface map is accurate to current Phase 3/3.17 reality.
- ADR linkage is complete at Analyst granularity (ADR-007/010/012 explicitly anchored).
- No contradictory claims against ARCH v1.2 or Phase 3 review history.

#### ROADMAP Phase 4 deliverable coverage map

All 8 roadmap deliverables map to >=1 functional requirement:

1. Curator worker -> F-4.1, F-4.2, F-4.22
2. Success-rate tracking -> F-4.3
3. Duplicate consolidation -> F-4.4, F-4.5, F-4.6
4. Auto-archive -> F-4.8, F-4.9
5. Skill self-improvement -> F-4.7, F-4.20
6. SOP conflict detection -> F-4.10, F-4.11
7. Work-pattern modeling -> F-4.19, F-4.20
8. Weekly retrospectives -> F-4.17, F-4.18

No deliverable coverage gap found.

#### DEBT carry-forward mapping verification

Phase 3 carry-forward entries in scope (`#72/#73/#75/#76/#77/#87/#88/#89/#90/#91`) are:
- explicitly listed in `phase-4/DEBT.md`
- mapped in `phase-4/requirements.md` via direct closure statements and F-4.26 accountability rule

Mapping is complete; no orphaned in-scope debt id.

#### Phase 3 -> Phase 4 interface fidelity

Requirements and INPUTS correctly reference the key Phase 3 surfaces at contract level:
- production extraction handler foundation (story 3.17 outcome)
- matcher/session_search/tool surfaces
- V010 lineage and project-scope/filter semantics

No hand-wave that rises to M+ severity at Analyst stage.

### C) Cross-phase regression check

- No iter-1 changes violate `ARCHITECTURE.md` v1.2 contracts.
- `spec-check` hook expectations remain satisfied.
- Phase 3 carry-forward expectations remain explicit and consistent.

### D/E) Iter-2 action and saturation

- **New iter-2 findings**: 0 (M+), 0 (L)
- **Inline fixes applied**: none required
- **DEBT additions**: none

**Iter-2 saturation note**: Hardening is complete for Analyst pass; this spec set is ready for
BMAD Architect dispatch.

---

## REVIEW iter-1 (Claude, 2026-05-18) — Architect pass

Scope: Codex Architect output at `aebd0ad` — 1013-line architecture.md with §1-§13
substantive coverage, 20/20 OPEN_QUESTIONS resolved, V011 schema sketch, type
sketches, coverage map. Reviewed for security / spec-consistency / simplicity /
readability against requirements + Phase 3 precedent.

### Findings summary

| # | Severity | Category | Title |
|---|---|---|---|
| A-IT1-F1 | **M** | spec-consistency | Cross-table timestamp format drift (ISO-8601 vs unixepoch microseconds) |
| A-IT1-F2 | **M** | forward-compat | 10 new tables lack nullable tenant_id for Phase 5 multi-user backfill |
| A-IT1-F3 | **M** | event taxonomy | F-3.8 vocab expansion to include curation_decision needs explicit ARCH update |
| A-IT1-F4 | L | dependency justification | 3 new crates listed without per-crate rationale |
| A-IT1-F5 | L | config rigor | CuratorConfig single field vs NFR-4.6 two-threshold pattern |
| A-IT1-F6 | L | budget arithmetic | §7 cycle budget depends on undocumented amortization |
| A-IT1-F7 | **M** | security | Adversarial Curator confidence-manipulation scenario unaddressed |
| A-IT1-F8 | L | failure handling | F-4.22 OOM conflates process-OOM and batch-OOM |
| A-IT1-F9 | L | testing rigor | F-4.21 warm-loop benchmark description thinner than Phase 3 precedent |
| A-IT1-F10 | L | schema integrity | playbook_revisions.parent_revision_id missing FK constraint |

### M-severity fixes applied inline

**F1 (timestamp format)** — replaced 8 occurrences of
`strftime('%Y-%m-%dT%H:%M:%fZ','now')` ISO-8601 with
`CAST(unixepoch('subsec') * 1000000 AS INTEGER)` microsecond INTEGER to match
Phase 3 V010 + ARCH §2.1 events table. Also flipped column types
(`week_start TEXT` → `week_start INTEGER` etc.) on 6 nullable timestamp columns.
Cross-table time-ordered queries (e.g., "show me curator_decisions joined with
events ordered by timestamp") now work without per-row conversion. Operators see
human-readable form via CLI/UI conversion at display time.

**F2 (tenant_id forward-compat)** — added nullable `tenant_id TEXT` to 10 new
Phase 4 tables: playbook_revisions, playbook_revision_outcomes,
curator_decisions, curator_review_queue, sop_conflicts, knowledge_items,
datasource_items, weekly_retrospectives, retrospective_citations,
curator_search_index. Phase 4 writers write NULL (single-operator scope); Phase 5
multi-user flips to NOT NULL with backfill per the V009 → Phase 5 ADR-013 pattern.
Without this, Phase 5 needs destructive migration of every Phase 4 table; with
it, Phase 5 is a single ALTER + backfill per table.

**F3 (event taxonomy expansion)** — added explicit §4.5 note pinning that
F-3.8's `Skill.kind` vocab is expanding from {match, injection, outcome} to
{match, injection, outcome, curation_decision}. The V011 atomic-slice ADR-013
must document this taxonomy expansion alongside the schema bump, so downstream
consumers (event-stream replay, session search, dashboards) treat the new kind
as first-class. Without this pin, Phase 3 consumers would treat
`curation_decision` as unknown.

**F7 (adversarial Curator)** — added §9.1 with compositional confidence-scoring
bounds. The attack: malicious LLM output yielding high confidence pushes an
auto-merge/auto-archive past the F-4.25 / §12.19 review threshold (>0.75)
without human gating. The defense:
- Confidence composed from deterministic signals (FTS overlap, structural diff,
  Jaccard, recency) + bounded LLM contribution (max +0.45)
- LLM-alone cannot push past auto-apply threshold (0.45 < 0.75 - lowest floor)
- Hard short-circuit to review queue when deterministic floor < 0.30
- Adversarial input itself blocked by Phase 3 F-3.13 + Curator's reuse of Phase 3
  redaction helpers

### L-severity findings (DEBT-seeded for Phase 4 implementation polish)

- **F4** — DEBT #97: per-crate justification for `cron`/`ordered-float`/`lru`
  before adoption. cron may be over-engineered for simple weekly cadence;
  ordered-float may be avoidable if score comparisons happen at decision sites
  not in collection keys.
- **F5** — DEBT #98: split CuratorConfig.embedding_budget_percent_cap into
  soft_cap_pct (0.08) + hard_breaker_pct (0.12) to match NFR-4.6's two-threshold
  pattern.
- **F6** — DEBT #99: §7 budget arithmetic depends on retrospective being "weekly
  amortized" — pin the amortization formula explicitly (sum of subcomponents
  3600ms per cycle excluding retrospective, +1200ms once weekly).
- **F8** — DEBT #100: F-4.22 OOM handling should distinguish process-OOM (panic
  → task aborts, supervisor restarts) from batch-OOM (controlled retry with
  reduced batch). Current text conflates them.
- **F9** — DEBT #101: F-4.21 phase4_warm_full_loop_benchmark needs concrete
  setup description in §11.3 (mirror Phase 3 phase3_warm_benchmark's seed →
  cold → curator → warm → assert pattern).
- **F10** — DEBT #102: add FOREIGN KEY(parent_revision_id) REFERENCES
  playbook_revisions(id) ON DELETE SET NULL to playbook_revisions table for
  graph consistency.

### Iter-1 conclusion

4 M-severity findings fixed inline; 6 L-severity findings DEBT-tracked.
Architecture is substantially correct and Phase 3-consistent after iter-1.

Recommend dispatching Codex iter-2 for cross-verification (similar to Phase 3
iter-2 saturation pattern). If iter-2 finds no M+ issues, hand off to BMAD PM
for story breakdown.

## REVIEW iter-2 (Codex, 2026-05-18) — Architect pass

Scope: Claude Architect iter-1 at `91e9e0c` + current Phase 4 architecture/docs.

### A) Grade Claude iter-1 findings (F1-F10)

Verdict: **10/10 agree**, **0 disagree**.

- `A-IT1-F1` (M): agree with root cause. Timestamp normalization to microsecond INTEGER is correct.
  Follow-up from this iter found 5 remaining `created_at TEXT` columns still carrying INTEGER
  defaults; fixed inline below.
- `A-IT1-F2` (M): agree. Forward-compat `tenant_id` columns are correctly added on all 10 new
  tables. No additional table miss found.
- `A-IT1-F3` (M): agree. Skill taxonomy expansion pin in §4.5 is correct and ADR-013/ARCH update
  requirement is explicitly called out.
- `A-IT1-F7` (M): agree. §9.1 adversarial confidence composition is directionally solid; 0.45 LLM
  cap and 0.30 deterministic floor are reasonable baseline controls for Phase 4.
- `A-IT1-F4/F5/F6/F8/F9/F10` (L): dispositions are reasonable as debt-seeded implementation polish.

### B) Independent re-audit

#### New M findings from iter-2 (fixed inline)

1. `A-IT2-F1` (M) — residual timestamp type drift in V011 SQL sketch.
- Evidence: five tables still had `created_at TEXT NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000000 AS INTEGER))`.
- Risk: mixed TEXT/INTEGER timestamp semantics breaks deterministic cross-table sort and violates
  ARCH v1.2 timestamp convention.
- Fix: changed these `created_at` columns to `INTEGER` in:
  - `curator_decisions`
  - `sop_conflicts`
  - `knowledge_items`
  - `datasource_items`
  - `curator_search_index`

2. `A-IT2-F2` (M) — F-4.5 embedding wiring lacked concrete endpoint/model/formula/cache pin.
- Evidence: architecture stated "embedding slot wiring" but omitted explicit API contract and blend
  formula.
- Risk: PM stories could re-derive incompatible wiring, scoring, and caching behavior.
- Fix: added explicit contract in §4.2:
  - Bifrost embeddings endpoint (`POST /v1/embeddings`)
  - default model profile (`text-embedding-3-small`, env-overridable)
  - blend formula and lexical fallback formula
  - cache strategy (in-process LRU + optional SQLite warm cache)

#### Additional audit points

- §11 testing strategy: adequate at architecture level; L-severity benchmark-detail debt (#101)
  remains valid and already tracked.
- §12 OPEN_QUESTIONS #4-#20: all are resolved with options + rationale and no unresolved scope gaps.
- §13 coverage map: all 26 F and 8 NFR are mapped; no missing row.
- V011 backfill ordering: safe and transactional; FTS rebuild scope is correctly limited to
  `playbooks_fts` (curator FTS surfaces are net-new, no rebuild needed).
- CLI surfaces (§4.6): story-slice granularity is sufficient (`status/run/review list/approve/reject/suppress`).
- Type sketches: `WeeklyRetrospective` timestamps now aligned to schema integer semantics.

### C) Cross-phase regression check

- Phase 3 surface contracts remain respected:
  - extraction ownership stays with PlannerSlotExtractionHandler;
  - matcher/injector are revision-aware consumers;
  - session_search reuse is additive via citation overlay.
- No silent violation identified against ARCHITECTURE.md v1.2 constraints at this spec stage.
- Tool-catalog count unaffected (no new LLM-callable tools introduced in this slice).

### D/E) Iter-2 action and verdict

- **Agree/disagree on Claude iter-1 findings**: 10 agree / 0 disagree.
- **New iter-2 findings**: 2 (both M), both fixed inline in architecture.md.
- **New DEBT seeded**: none (no new deferrals introduced).

Saturation verdict: Architect hardening is now in good shape for PM dispatch.

---

## REVIEW iter-1 (Claude, 2026-05-18) — PM pass

Scope: Codex PM output at `1e11a0f` — 22 stories + requirements.md §4 table.
Reviewed coverage matrix (every F-4.x + NFR-4.x → ≥1 story), dependency
correctness, and story format compliance against Phase 3 PM precedent.

### Findings summary

| # | Severity | Category | Title |
|---|---|---|---|
| P-IT1-F1 | **M** | coverage gap | NFR-4.4 (telemetry retention + compaction + storage cap) → 0 stories |
| P-IT1-F2 | L | dep transitivity | story 4.22 close-out deps `4.2-4.21` skips explicit 4.23 (the new story); fix-up edited |

### F1 (M, FIXED) — NFR-4.4 zero-story coverage

**Evidence** — coverage scan over `stories/story-4.*.md` for each NFR-4.X:
- NFR-4.1 → 2 stories
- NFR-4.2 → 1 story
- NFR-4.3 → 1 story
- **NFR-4.4 → 0 stories** ← gap
- NFR-4.5..NFR-4.8 → 1-3 stories each

NFR-4.4 mandates 90-day hot retention + 300 MB/month/project cap + tiered
raw→summarized compaction (per §12.12 resolution option B). Without a story
owning the retention/compaction runtime, Phase 4 emits substantial telemetry
(`curator_*` Misc, `Skill{kind:"curation_decision"}`, `curator_decisions`
ledger, `weekly_retrospectives` + `retrospective_citations`,
`curator_search_index`) with no compaction tail — the per-project SQLite DB
grows unbounded and the 300 MB cap fails operationally even if mentioned in
prose.

Story 4.12 (Curator telemetry schema) was the closest fit but covers emit
mechanics only (event payload shape, taxonomy validation); extending it to
include retention runtime would push it well past the 3h estimate ceiling.

**Fix applied** — added new **story 4.23 — Curator telemetry retention +
compaction** (2.5h, deps 4.2 + 4.12). Owns the 90-day window, tiered
raw→summarized compaction, cap-exceeded trigger, atomic per-batch commits
(NFR-4.3 reuse), and idempotent re-run semantics. Updated requirements.md
§4 table + story 4.22 close-out deps to include 4.23.

This mirrors the Phase 3 story 3.17 post-hoc-fix pattern, but caught BEFORE
execution starts (vs Phase 3's post-execution discovery in REVIEW iter-3).
Recovery cost is minimal — no rework of already-shipped code.

### F2 (L, FIXED inline) — story 4.22 dep range needed extension

Updated requirements.md §4 row for 4.22 from `4.2-4.21` → `4.2-4.21, 4.23`
to capture the new dep.

### Iter-1 conclusion

- 1 M-severity finding fixed by adding story 4.23.
- 1 L-severity finding fixed inline (dep update on 4.22).
- All 26 F-4.x and 8 NFR-4.x now have ≥1 story coverage (verified via grep).
- Dependency graph remains acyclic: 4.1 → 4.2 → others; close-out 4.22 waits
  on all others.
- Story format matches Phase 3 precedent (Status/Estimated/Dependencies/Phase/Type
  header; Goal/Acceptance/Non-goals/Implementation/Verification/Refs structure).
- Each load-bearing component (4.3-4.10) story explicitly requires production
  impl + main.rs wiring + integration test (Phase 3 story 3.17 lesson applied).
- Required regression tests from architecture §11 all present (4.15-4.20).

Recommend dispatching Codex iter-2 for cross-verification (Phase 3 precedent —
Codex iter-2 caught 2 additional M-issues Claude missed). If iter-2 saturates,
hand off to first GSD execute-story dispatch (story 4.2 — V011 atomic slice).

## REVIEW iter-2 (Codex, 2026-05-18) — PM pass

Scope: Claude PM iter-1 at `bc42b33` + current 23-story Phase 4 plan.

### A) Grade Claude iter-1 findings

Verdict: **2/2 agree**, **0 disagree**.

- `P-IT1-F1` (M, NFR-4.4 coverage gap): agree with severity and root cause. Story 4.12 covers
  telemetry emission/taxonomy but does not own retention-window + compaction + storage-cap runtime
  behavior required by NFR-4.4. Adding story 4.23 is the correct structural fix.
- `P-IT1-F2` (L, 4.22 dependency update): agree. Updating 4.22 deps to include 4.23 is correct.

Story 4.23 rigor check:
- Scope is concrete and testable (retention scheduler, compaction path, cap trigger, atomicity,
  idempotency, observability).
- Implementation plan has 9 actionable steps and maps cleanly to architecture §3.2/§12.12.

### B) Independent re-audit

#### DEBT carry-forward mapping

Verified all Phase 3 carry-forward ids are tagged to Phase 4 stories:
- `#72/#73/#75/#76/#77/#87/#88/#89/#90/#91` -> mapped in story refs (including close-out 4.22).

#### Phase 4-introduced DEBT mapping

Verified all `#92-#102` are tagged to at least one Phase 4 story (directly or close-out 4.22).
No orphan DEBT id found.

#### Story format compliance

All 23 stories include required structure:
- header fields (`Status/Estimated/Dependencies/Phase/Type`)
- sections (`Goal/Acceptance criteria/Non-goals/Implementation steps/Verification/Refs`)

#### Verification-block compliance

All 23 stories include full gate commands:
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo test --workspace`
- `bash scripts/spec-check.sh`

#### Load-bearing story checks (4.3-4.10)

Each load-bearing story requires:
- production runtime implementation (not test-only trait scaffold)
- runtime wiring through `main.rs` / CuratorWorker graph
- integration test exercising end-to-end behavior with stubs where needed

#### Dependency graph

No cycle found. Flow remains DAG:
- foundation `4.1 -> 4.2`,
- runtime/component stories fan out,
- close-out `4.22` waits on `4.2-4.21, 4.23`.

#### Total estimate sanity

23 stories total estimate is ~54.5 hours, which is reasonable for a 3-week Phase 4 execution window
with 1-3h story granularity.

#### V011 atomic-slice story check (4.2)

Story 4.2 acceptance still pins all required atomic pieces:
- V011 SQL migration,
- ADR-013,
- ARCH bump/reconciliation,
- event taxonomy expansion,
- FTS trigger semantics,
- backfill behavior.

### C) Cross-phase consistency

Story 4.23 is a clean pre-execution gap fix (not a conflicting overlap):
- Story 4.12 owns telemetry emission/taxonomy.
- Story 4.23 owns retention/compaction lifecycle for emitted data.

This avoids repeating Phase 3's post-execution structural-gap recovery pattern.

### D/E) Iter-2 action and verdict

- **New iter-2 findings**: 0 (M+), 0 (L)
- **Inline fixes applied**: none
- **New DEBT seeded**: none

Saturation verdict: PM hardening is complete; proceed to GSD execute-story dispatch starting with
story 4.2 (V011 atomic slice).
