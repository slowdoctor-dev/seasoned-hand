# Phase 4 — Curator + Self-Improvement

Date: 2026-05-18  
Owner: BMAD Analyst pass

## 1. Goals (what success looks like)

- Close the loop from verified execution artifacts to continuously improved playbooks without
  requiring manual librarian work on each task.
- Ensure curation decisions are safe-by-default: no destructive consolidation/archive action can
  silently degrade production task quality.
- Make Curator behavior observable and measurable so Phase 4 can be tuned with evidence instead
  of anecdotal impressions.
- Deliver measurable week-over-week improvement signals that match roadmap intent: better match
  precision, lower stale-playbook ratio, and fewer contradictory procedures.
- Preserve Phase 0-3 reliability contract: Curator failures cannot block primary task execution.

## 2. Non-functional requirements

- `NFR-4.1` **Curator decision latency bound**  
  For a single curation cycle with <= 50 candidate artifacts, p95 runtime <= 4.0s, p99 <= 8.0s,
  measured on the project baseline environment.  
  Why this matters: if curation latency explodes, background learning starves and weekly
  retrospective cadence collapses.

- `NFR-4.2` **Task-path isolation**  
  Curator path must be fully asynchronous from user task completion and may not increase
  end-to-end verifier `PASS` latency by > 50ms p95.  
  Why this matters: learning value is unacceptable if core execution gets slower or flaky.

- `NFR-4.3` **Archive/consolidation atomicity**  
  Any multi-row decision (merge/archive/relink) must commit atomically; no partial state may be
  externally visible.  
  Why this matters: partial writes create irrecoverable drift and broken provenance chains.

- `NFR-4.4` **Telemetry retention budget**  
  Curator telemetry and retrospectives must keep 90-day hot retention in SQLite by default with
  bounded storage growth <= 300MB/month/project under "expected load profile" defined as
  100 verified task-complete artifacts/week + 4 weekly retrospective generations + 28 daily
  curator cycles. Architect may adjust the load profile constants based on dogfood data
  but must preserve the per-project bound.  
  Why this matters: no retention means no tuning; unbounded retention breaks local-first footprint.

- `NFR-4.5` **Conflict alert SLA**  
  SOP conflict detection events generated within 24h of conflicting evidence arrival; alert drop
  rate < 1% over rolling 30 days.  
  Why this matters: stale conflict signals convert into wrong guidance injection.

- `NFR-4.6` **Embedding-slot cost ceiling**  
  Curator embedding + rerank calls must stay <= 8% of total monthly token spend by default
  policy; hard circuit breaker at 12%. For projects with zero or near-zero baseline spend
  (setup/empty state), an absolute fallback budget of 50_000 embedding tokens/month applies
  until baseline establishes (defined as ≥ 7 days of token spend > 1_000 tokens/day).  
  Why this matters: Phase 4 cannot make cost unpredictable or erase Bifrost efficiency gains;
  zero-baseline projects need a startup runway.

- `NFR-4.7` **False-positive safety bounds**  
  Auto-archive and auto-merge false-positive rate each <= 2% on audited sample set with
  minimum size: N=100 archive decisions + N=100 merge decisions, sampled across at least
  3 representative project corpus shapes (small/medium/large by artifact count). Architect
  may broaden sampling but must preserve the minimum-size floor.  
  Why this matters: wrong automation decisions destroy trust in learned playbooks; sample
  too small to detect 2% rate (50+ samples needed to distinguish 2% from 5% at 95% CI).

- `NFR-4.8` **Retrospective cadence reliability**  
  Weekly retrospective generation success >= 99% across active projects; failed runs retried within
  6h with idempotent semantics.  
  Why this matters: retrospectives are the Phase 4 control loop; missed weeks stall improvement.

## 3. Functional requirements

- `F-4.1` **Curator worker runtime surface**  
  Introduce a production Curator worker orchestration surface (startup, schedule, shutdown,
  health signals) that consumes Phase 3 learning artifacts.  
  Contract pin: Curator is best-effort and never blocks verifier PASS/FAIL completion path.  
  Why this matters: without a real runtime owner, Phase 4 repeats the Phase 3 structural gap.

- `F-4.2` **Curator cycle trigger model**  
  Curator must support both interval-triggered and backlog-triggered cycles with deterministic
  tie-breaking for artifact ordering.  
  Architect pins scheduling internals; Analyst pins dual-trigger scope.  
  Why this matters: interval-only creates stale backlog; backlog-only can thrash.

- `F-4.3` **Success-rate tracking per playbook revision**  
  Track rolling success/failure metrics at playbook revision granularity, not only playbook id.
  Includes decay/window semantics. Revision identity follows F-4.7 / OPEN_QUESTIONS #10
  resolution — the same revision key must work across F-4.3 (counters), F-4.6 (consolidation),
  and F-4.20 (recommendation provenance).  
  Why this matters: without revision-aware metrics, Curator cannot detect regressions after edits;
  diverging revision keys across F-4.3/4.6/4.7/4.20 split the learned-graph state.

- `F-4.4` **Duplicate candidate discovery pipeline**  
  Build candidate sets for potential duplicates using Phase 3 FTS + matcher features; include
  deterministic fallback when embeddings unavailable.  
  Closes DEBT #76 partially (FTS weight retune context surfaced).  
  Why this matters: no candidate generation means consolidation never runs in practice.

- `F-4.5` **Embedding rerank for consolidation**  
  Add embedding-based rerank stage over duplicate candidates before merge/archive decision.
  Closes DEBT #72.  
  Why this matters: lexical overlap alone merges unrelated procedures too often.

- `F-4.6` **Consolidation decision policy**  
  Define policy for merge vs keep-separate vs quarantine with confidence thresholds and mandatory
  provenance links from source artifacts to resulting revision(s).  
  Architect pins exact scoring function and threshold storage shape.  
  Why this matters: consolidation without explicit policy is untestable and unsafe.

- `F-4.7` **Playbook revisioning behavior**  
  Skill self-improvement must produce revisioned updates or versioned forks
  (see OPEN_QUESTIONS #10 — Analyst pins the choice between A/B/C/D; Architect
  resolves schema/migration semantics), never in-place destructive overwrite without
  rollback metadata. Whatever shape is chosen, F-4.3 success-rate tracking and F-4.6
  consolidation policy must reference the same revision identity (so the matcher /
  counter / injector chain has a stable referent).  
  Why this matters: irreversible edits prevent incident recovery; conflicting revision
  identity between F-4.3/F-4.6/F-4.7 splits the learned-graph state.

- `F-4.8` **Auto-archive policy engine**  
  Implement stale/low-signal archive recommendations and optional auto-archive execution tied to
  confidence + guardrails. Interaction with Phase 3 `playbooks.status='archived'` soft-delete
  vocabulary is pinned by OPEN_QUESTIONS #11 (auto-archive may either reuse the same 'archived'
  status or use a distinct value like 'auto_archived' for audit traceability — Architect resolves).  
  Closes DEBT #90 (dedup/archive safety guard).  
  Why this matters: unmanaged library growth degrades matcher precision and injection quality;
  blurring auto-archive with operator-initiated archive hides intent in audit trails.

- `F-4.9` **Archive reversibility surface**  
  Provide deterministic unarchive/restore path preserving prior ranking metadata and provenance.
  Why this matters: operators need escape hatch when automation makes wrong calls.

- `F-4.10` **SOP conflict detection**  
  Detect contradictory SOP/procedure guidance across active playbooks and emit conflict artifacts
  with evidence references. Minimum baseline (Architect may extend, see OPEN_QUESTIONS #9):
  (a) structural-step diff (overlapping `## Procedure` step prefixes with divergent guidance),
  plus (b) LLM-judged semantic contradiction over candidate pairs flagged by (a). Both signals
  must agree before raising a conflict above the lowest severity tier (see F-4.11).  
  Closes DEBT #89 partially (prompt specificity feeds conflict quality).  
  Why this matters: contradictory guidance can push planner into oscillation or unsafe steps;
  un-baselined algorithms ship as architect homework and risk silent over/under-detection.

- `F-4.11` **Conflict severity triage**  
  Classify conflicts by severity and confidence to reduce alert fatigue; include suppress/mute
  policy surface.  
  Why this matters: noisy conflict streams are ignored and become operationally useless.

- `F-4.12` **Knowledge + Datasource write-paths**  
  Implement production write paths for Knowledge and Datasource artifact kinds with explicit emit
  conditions and schema persistence.  
  Closes DEBT #73 (writer completion), contributes to #77 (source-project denorm).  
  Why this matters: missing writers means large parts of the learned graph never exist.

- `F-4.13` **L2 cross-source verification rollout**  
  Enforce L2 policy for cross-source corroboration on designated artifact classes; include phased
  rollout control for project safety.  
  Closes DEBT #73 (L2 enforcement).  
  Why this matters: single-source contamination propagates bad procedures.

- `F-4.14` **Source project denormalization**  
  Persist source-project linkage fields required for fast project-scope filtering and multi-project
  analytics.  
  Closes DEBT #77.  
  Why this matters: joins at query time become too expensive and brittle as corpus grows.

- `F-4.15` **Curator telemetry schema**  
  Add first-class telemetry events for candidate generation, merge/archive decisions, suppression,
  retry, and cost usage.  
  Closes DEBT #88 (event-kind documentation parity companion).  
  Why this matters: no telemetry means no explainability and no tuning loop.

- `F-4.16` **Event taxonomy reconciliation**  
  Curator events must conform to architecture event-kind conventions and naming consistency,
  including extraction-prefix boundaries from Phase 3.  
  Closes DEBT #88.  
  Why this matters: taxonomy drift breaks search, dashboards, and review tooling.

- `F-4.17` **Weekly retrospective generation**  
  Generate weekly project retrospectives summarizing wins/failures/conflicts/changes with clear
  provenance and confidence annotations.  
  Closes roadmap deliverable and DEBT #75 partially.  
  Why this matters: retrospectives are the human-auditable output of autonomous learning.

- `F-4.18` **Retrospective quality hardening**  
  Apply anti-hallucination constraints, citation requirements, and refusal pathways for missing
  evidence in retrospective generation prompts/validators.  
  Closes DEBT #75 and #89 partially.  
  Why this matters: fabricated retrospectives cause wrong strategic decisions.

- `F-4.19` **Work-pattern model extraction**  
  Derive recurring task/workflow patterns from event-stream evidence and rank by recency/value.
  Architect pins model representation and persistence format.  
  Why this matters: without pattern abstraction, learning remains one-off and non-compounding.

- `F-4.20` **Pattern-to-playbook recommendation loop**  
  Curator must surface actionable recommendations from detected patterns to playbook revisions or
  new playbook proposals with decision trace.  
  Why this matters: pattern detection alone has no product value.

- `F-4.21` **Embedding warm benchmark full-loop**  
  Add phase-level benchmark that validates warm-path improvement with Curator-enabled corpus
  evolution and reports delta vs cold baseline.  
  Closes DEBT #87.  
  Why this matters: claimed self-improvement must be measured with loop in place.

- `F-4.22` **Curator failure containment**  
  Curator must quarantine the decision unit and continue remaining cycle work on ANY of:
  Rust panic / propagated error, LLM refusal, malformed payload, timeout (NFR-4.1 latency
  bound exceeded), out-of-memory, SQLite DB lock contention (BUSY error after retry budget
  exhausted), or slot-router resolution failure (e.g. `embedding` slot misconfigured).
  Each quarantine emits a Misc telemetry event with `kind` + `failure_category` discriminant.  
  Why this matters: one malformed artifact cannot freeze the whole learning program; narrow
  failure-mode coverage (exception-only) misses the operationally common cases.

- `F-4.23` **Config + feature-flag controls**  
  Provide explicit runtime toggles for curator enablement, auto-archive, auto-merge, and
  retrospective generation with strict env parsing behavior.  
  Closes DEBT #91.  
  Why this matters: ambiguous toggles create accidental behavior in production.

- `F-4.24` **Project-scope isolation guarantees**  
  Curator decisions must never cross project boundary by default; any multi-project analysis is
  opt-in and labeled experimental for Phase 4.  
  Why this matters: accidental cross-project leakage violates trust and future tenant model.

- `F-4.25` **Operator review queue**  
  Add human-review queue for low-confidence merge/archive/conflict decisions with explicit accept /
  reject feedback path feeding future policy tuning.  
  Why this matters: safe autonomy needs bounded human correction points.

- `F-4.26` **Phase 4 close-out accountability mapping**  
  Every inherited Phase 3 DEBT entry in scope (#72/#73/#75/#76/#77/#87/#88/#89/#90/#91) must map
  to a closed, partially closed, or explicitly deferred state with evidence in Phase 4 close-out.
  Why this matters: debt without closure mapping repeats cross-phase drift.

## 4. Story breakdown

| story_id | title | est | deps | status |
|---|---|---:|---|---|
| 4.1 | Phase 4 scaffolds (retroactive doc baseline) | 0.5h | — | done |
| 4.2 | Atomic slice: V011 + ADR-013 + ARCH v1.3 + taxonomy/FTS/backfill | 3.0h | 4.1 | done |
| 4.3 | CuratorWorker production runtime + main.rs wiring | 3.0h | 4.2 | done |
| 4.4 | CandidateBuilder + EmbeddingReranker production wiring | 3.0h | 4.2, 4.3 | done |
| 4.5 | ConsolidationEngine with revision-chain writes | 3.0h | 4.2, 4.4 | done |
| 4.6 | ConflictDetector production implementation | 2.5h | 4.2, 4.4 | done |
| 4.7 | RetrospectiveGenerator weekly cadence + citation validator | 3.0h | 4.2, 4.3, 4.6 | done |
| 4.8 | WorkPatternExtractor + recommendation loop | 2.5h | 4.2, 4.3 | done |
| 4.9 | OperatorReviewQueue + curator CLI surfaces | 3.0h | 4.2, 4.3, 4.5, 4.6 | done |
| 4.10 | Knowledge/Datasource writers + L2 enforcement rollout | 3.0h | 4.2, 4.3, 4.4 | done |
| 4.11 | Curator failure containment + adversarial confidence bounds | 2.5h | 4.3, 4.4, 4.5, 4.6 | done |
| 4.12 | Curator telemetry schema + event taxonomy reconciliation | 2.0h | 4.2, 4.3, 4.5, 4.6, 4.7, 4.8 | done |
| 4.13 | Auto-archive policy engine + reversibility | 2.5h | 4.5, 4.9, 4.12 | done |
| 4.14 | Strict SH_CURATOR_* config parsing + feature flags | 2.0h | 4.3 | done |
| 4.15 | Project-scope isolation enforcement regression | 1.5h | 4.2, 4.3, 4.5, 4.10 | done |
| 4.16 | NFR-4.7 false-positive audit harness | 2.5h | 4.5, 4.13 | done |
| 4.17 | Revision-chain integrity regression | 2.0h | 4.2, 4.5 | done |
| 4.18 | curator_search_fts maintenance-trigger correctness regression | 1.5h | 4.2 | done |
| 4.19 | Review queue state-machine regression + suppression TTL | 1.5h | 4.9 | done |
| 4.20 | Embedding cost circuit-breaker regression | 2.0h | 4.4, 4.14 | done |
| 4.21 | phase4_warm_full_loop_benchmark | 3.0h | 4.3, 4.4, 4.5, 4.7, 4.8, 4.10, 4.12, 4.13 | ready |
| 4.22 | Phase 4 acceptance gate + close-out | 2.5h | 4.2-4.21, 4.23 | ready |
| 4.23 | Curator telemetry retention + compaction (NFR-4.4 close) | 2.5h | 4.2, 4.12 | ready |

## 5. Acceptance criteria (Phase-level)

Phase 4 is accepted when all of the following hold:

1. One-month operational proxy is defined as: 4 consecutive weekly curator cycles in CI replay plus
   at least 200 verified task-complete artifacts in representative fixture corpus, with total
   wall-clock CI budget ≤ 45 min on baseline runner (per-artifact replay amortizes via stub LLM +
   pre-rendered transcripts; Architect may relax with rationale if the bound is genuinely
   infeasible).
2. Under that proxy corpus, active playbook precision@3 improves by >= 15% vs Phase 3 baseline, and
   stale-playbook ratio drops by >= 25% without raising regression failure rate.
3. Auto-archive and auto-merge safety audits meet NFR-4.7 bounds.
4. Weekly retrospectives generate on schedule with citation coverage >= 95% and no uncited factual
   assertions in acceptance audit set.
5. Curator failures are isolated and do not degrade Phase 0-3 primary task completion flow.
6. Inherited Phase 3 debt mapping required by F-4.26 is completed with evidence links.

## 6. Out of scope (explicitly deferred)

- Multi-user / cross-tenant curator policy arbitration and shared-memory marketplace behavior
  (Phase 5).
- Public OSS curator policy plug-in API and third-party extension surface (Phase 6).
- Full graph database migration for long-term memory (Phase 5+ exploration).
- Autonomous fine-tuning loop over model weights (non-goal; excluded by philosophy constraints).
- General conversational memory unrelated to verified task execution (non-goal continuity).
- Full automatic deletion workflows for compliance-grade data lifecycle management (Phase 5+).

Debt mapping deferrals:
- DEBT #76 may remain partial if rerank lands but full FTS weight retune not yet converged.
- DEBT #91 may remain partial if strict parsing lands for curator flags only, pending global config
  harmonization in Phase 5.

## 7. Risks and mitigations

- False-positive auto-archive hides valuable playbooks.  
  Mitigation: confidence thresholds, review queue, reversible archive, audit sampling.
- Over-aggressive consolidation merges dissimilar procedures.  
  Mitigation: embedding rerank, structural diff checks, quarantine bucket, manual approval band.
- Retrospective hallucination or fabricated claims.  
  Mitigation: citation-required prompt/validator, refusal-on-missing-evidence, bounded templates.
- Work-pattern model bias toward high-volume but low-value tasks.  
  Mitigation: weighted scoring with outcome quality and recency, not raw frequency.
- Embedding spend spikes beyond budget.  
  Mitigation: candidate prefilter, cap per cycle, cost circuit breaker (NFR-4.6).
- SOP conflict detector creates high-noise alert streams.  
  Mitigation: severity tiers, suppression, dedup windows, on-call-friendly digesting.
- Curator crash loop interferes with runtime stability.  
  Mitigation: isolate worker process/task, backoff, quarantine failures, health watchdog.
- Telemetry retention growth exceeds local storage limits.  
  Mitigation: TTL/compaction policy, tiered summaries, 90-day default cap.
- Phase 3 debt closure gets fragmented and unverifiable.  
  Mitigation: enforce F-4.26 closure matrix in PM stories and close-out checklist.
- Policy drift between requirements and implementation.  
  Mitigation: same-PR spec/code reconciliation rule inherited from AGENTS §8.

## 8. Dependencies (external + internal)

Internal dependencies:
- Phase 3 production extraction handler and learning artifact schema (V010 lineage).
- Matcher/session_search/tool surfaces introduced in Phase 3.
- Architecture v1.2 event taxonomy and learning surfaces.
- ADR anchors: ADR-007 conservative learning, ADR-010 process-control plan semantics,
  ADR-012 phase-3 reconciliation pattern.

Operational / likely additions:
- Embedding slot utilization in existing model routing (no new model class required, but cost
  budgeting required).
- Possible new crates for similarity math / stats windows / scheduling ergonomics (Architect to pin
  exact choices with ARCH addendum if new deps are introduced).

## 9. Open questions

See `/specs/phase-4/OPEN_QUESTIONS.md` for detailed alternatives and next-step ownership.
