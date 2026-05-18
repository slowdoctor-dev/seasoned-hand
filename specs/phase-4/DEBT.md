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

| Severity | Meaning | Default handling |
|---|---|---|
| H0 | Release-blocking correctness/safety issue | Must fix in current slice |
| H1 | High risk, user-visible correctness/regression | Fix this phase unless explicit ADR |
| H2 | Medium risk, bounded workaround exists | Plan in active phase with owner |
| H3 | Low risk or optimization | Schedule when capacity allows |

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
