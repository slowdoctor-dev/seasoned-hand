# Phase 4 — Inputs

Date: 2026-05-18  
Owner: BMAD Analyst pass

## 1. What Phase 4 ships (per ROADMAP)

ROADMAP Phase 4 defines these deliverables:

- Curator worker (scheduled background curation)
- Success-rate tracking across learned procedures
- Duplicate consolidation
- Auto-archive of stale/low-signal procedures
- Skill self-improvement loop
- SOP conflict detection
- Work-pattern modeling
- Weekly retrospectives

Analyst scope in this phase requirements must explicitly include all eight.

## 2. Core philosophy + decision records

Phase 4 must remain consistent with:

- Philosophy docs: `VISION.md`, `PRINCIPLES.md`, `NON_GOALS.md`
- ADR-001..ADR-012, with primary anchors:
- ADR-007 conservative learning guardrails (scope and safety boundaries)
- ADR-010 plan-as-process-control-block (structured lifecycle discipline)
- ADR-012 reconciliation pattern (spec/code drift must close in same slice)

Implication for Curator: autonomous improvement is required, but only under evidence-linked,
reversible, and observable controls.

## 3. Immutable architecture surfaces Phase 4 must respect

From ARCHITECTURE v1.2 baseline:

- Verification pipeline contract remains load-bearing (`ARCH` verifier sections).
- 12-slot model routing remains fixed shape; `embedding` slot is where semantic rerank cost lands.
- Learning artifacts and event-stream append-only principle remain core invariants.
- Multi-user isolation is still Phase 5 territory; Phase 4 defaults to project scope.

Phase 4 may extend around these surfaces, not replace them.

## 4. Schema reality (V010 baseline and gaps)

Current reality after Phase 3:

- V010 introduced learning artifacts foundation and search surfaces.
- Extraction handler now exists in production (story 3.17), closing prior structural blocker.
- Event taxonomies and telemetry are present but not yet curator-grade complete.

Known Phase 4 gaps to resolve in Architect pass:

- Curator decision tables / telemetry schema extensions
- revision-level success tracking persistence
- conflict artifact representation
- retrospective output storage + retention controls
- consolidation provenance model and rollback metadata

## 5. Code surfaces that touch Phase 4

Primary surfaces (from Phase 3 completion and review context):

- `crates/seasoned-hand-core/src/verifier/gate.rs`
- `crates/seasoned-hand-core/src/verifier/extraction.rs`
- `crates/seasoned-hand-core/src/verifier/extraction_handler.rs`
- `crates/seasoned-hand-core/src/events/session_search.rs`
- `crates/seasoned-hand-core/src/agent/init/injector.rs`
- `crates/seasoned-hand-core/src/agent/init/mod.rs`
- `crates/seasoned-hand-core/src/matcher/*`
- `crates/seasoned-hand-core/src/router/mod.rs`
- `crates/seasoned-hand-core/src/llm/mod.rs`
- `crates/seasoned-hand-server/src/main.rs`
- `crates/seasoned-hand-core/migrations/V010__phase3_learning_artifacts.sql`

Phase 4 likely introduces curator-specific worker/orchestrator modules and migration(s) V011+.

## 6. External research (Manus / Hermes signals)

Relevant research cues from `/specs/07-research/`:

- Faster inner loop requires durable learning feedback, not static memory capture.
- Weekly retrospective value comes from grounded, evidence-cited summaries.
- Global knowledge graph aspiration should be phased; Phase 4 should implement practical local
  curation primitives without premature graph-overhaul scope.

## 7. DEBT entries that name Phase 4 as pay-down

Inherited Phase 3 DEBT entries targeted by this phase:

- `#72` embedding rerank over FTS candidates
- `#73` Knowledge/Datasource writers + L2 cross-source enforcement
- `#75` summarizer/retrospective hardening
- `#76` FTS5 weight retune
- `#77` source_project_id denormalization
- `#87` warm benchmark with full learning loop
- `#88` event-kind documentation and taxonomy reconciliation
- `#89` LLM refusal prompt specificity hardening
- `#90` dedup/archive safety guard
- `#91` strict SH_LEARNING_ENABLED/config parsing behavior

Analyst requirement: each is mapped to a Phase 4 F-number or explicit deferral reason.

## 8. Open questions parked for the Architect

Analyst intentionally defers technical-shape specifics while pinning scope:

- Similarity metric blend and decision threshold materialization
- Consolidation write model (revision fork vs in-place lineage updates)
- Conflict detection algorithm details (textual/semantic hybrid)
- Retrospective generation slot, prompt envelope, and validator mechanics
- Telemetry table shape and compaction strategy

See `/specs/phase-4/OPEN_QUESTIONS.md`.

## 9. Cross-phase REVIEW context

Phase 3 REVIEW iterations established the critical lesson:

- Minimum-AC scaffolds can pass local checks while missing production glue.
- The extraction loop existed on paper but not in production until story 3.17.

Phase 4 operating rule:

- Each major deliverable must include runtime ownership + trigger + persistence + observability,
  not only interfaces/tests.

## 10. What the BMAD Analyst deliverable looks like

This pass delivers:

- High-density functional contract (22-30 F requirements)
- Measurable NFR envelope (6-9 NFRs)
- Explicit out-of-scope boundaries to protect Phase 4 focus
- Architect-facing open questions with real option tradeoffs
- Debt inheritance map ensuring cross-phase closure accountability

Output files:

- `requirements.md`
- `INPUTS.md`
- `OPEN_QUESTIONS.md`
- `DEBT.md`
