# Phase 5 — Technical Debt Ledger

Date: 2026-05-20  
Owner: BMAD Analyst seed

This ledger starts with inherited carry-forward items from Phase 4 close-out and records
Phase 5-specific shortcuts/deferrals during implementation.

## Inherited carry-forward targets from Phase 4

### #76 FTS5 weighting retune (PARTIAL)
- **Current state**: Seed weights landed; production telemetry-guided retune not fully closed.
- **Phase 5 expectation**: close or re-baseline with measured relevance outcomes in multi-user
  corpus.

### #91 Global config strict-parse harmonization (PARTIAL)
- **Current state**: Curator scope closed in story 4.14; non-curator config families remain mixed.
- **Phase 5 expectation**: close with global strict parse/fail-fast policy.

### #92 Adaptive auto-archive thresholds (H3)
- **Current state**: Static per-project thresholds shipped; adaptive policy deferred.
- **Phase 5 expectation**: close or explicitly defer with evidence-backed rationale.

### #93 Optional fork-promotion governance (H3)
- **Current state**: Revision-chain baseline shipped; optional governance mode deferred.
- **Phase 5 expectation**: decide optional governance path for shared org playbooks.

### #94 Retrospective tiered model-by-size policy (H3)
- **Current state**: Single summarizer profile baseline shipped.
- **Phase 5 expectation**: close or defer with cost/quality threshold criteria.

### #96 Curator rationale schema evolution tooling (H3)
- **Current state**: Structured rationale exists; schema-evolution tooling deferred.
- **Phase 5 expectation**: add compatibility/versioning tooling baseline.

### #97 Per-crate dependency justification discipline (H3)
- **Current state**: Documentation discipline deferred.
- **Phase 5 expectation**: close by enforcing per-crate justification + ARCH §1 addendum updates for
  net-new dependencies.

## Additional carry-forward from cross-phase security review

### #S-1 Tenant-scoped event redaction policy unresolved (NEW carry-in)
- **Source**: `specs/SECURITY_REVIEW.md` iter-3 observation (2026-05-20).
- **Current state**: Action/Observation events may carry raw tool args/outputs.
- **Phase 5 expectation**: resolve via explicit tenant-visible redaction/access policy and tests.

## Expected disposition in Phase 5 close-out

- Expected to close: `#91`, `#97`, `#S-1`.
- Expected to close or evidence-defer: `#76`, `#92`, `#93`, `#94`, `#96`.
- Any remaining open item requires successor owner (Phase 6) + explicit rationale in close-out
  matrix.

## Categories quick-reference (cross-phase mapping)

Phase 4 used H0/H1/H2/H3 labels; earlier phases used H/M/L. Maintain mapping consistency:

| Phase 4/5 style | Phase 0-3 style | Meaning |
|---|---|---|
| H0 | H | Release-blocking correctness/safety issue |
| H1 | H | High-risk user-visible regression/security risk |
| H2 | M | Medium-risk with bounded workaround |
| H3 | L | Low-risk optimization/documentation debt |

## Seed (TBD)

Architect/PM/execute-story passes append entries in this format:

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

This ledger intentionally focuses on inherited accountability and expected disposition. Concrete
implementation debt entries should be appended by Architect/PM and story execution passes.
