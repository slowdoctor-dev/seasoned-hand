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
