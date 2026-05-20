# Phase 5 — Technical Debt Ledger

Date: 2026-05-20  
Owner: BMAD Analyst seed

This ledger starts with inherited carry-forward items from Phase 4 close-out and records
Phase 5-specific shortcuts/deferrals during implementation.

## Inherited carry-forward targets from Phase 4

### #76 FTS5 weighting retune (PARTIAL) — PARTIAL CLOSED 2026-05-21 via story 5.24
- **Current state**: Seed weights landed; production telemetry-guided retune not fully closed.
- **Phase 5 expectation**: close or re-baseline with measured relevance outcomes in multi-user
  corpus.
- **Partial closure (story 5.24)**: Phase 5 lacks the production dogfood corpus needed for a
  meaningful retune (warm-loop benchmark's synthetic queries don't exercise the title/keyword/
  content balance the way real operator queries do). Per the story carve-out, partial close
  with explicit successor:
  - `crates/seasoned-hand-core/src/search/fts_weights.rs` now exposes named column-weight
    constants for `playbooks_fts`, `session_search_fts`, and `curator_search_fts` (all
    uniform 1.0 today — matches FTS5 default rank, preserves the Phase 4 warm-loop benchmark
    precision@3 floor). Future retunes touch one place rather than scattered `bm25()`
    literals across the matcher / session_search / curator queries.
  - `specs/phase-5/dogfood_fts_retune.md` documents the full measurement procedure (eval set
    requirements, relevance metric, grid-search shape, acceptance gate).
  - **Successor**: Phase 6 ROADMAP must carry "FTS5 weight retune with dogfood corpus" once
    Phase 6 architecture lands. Full close requires precision@3 ≥+3pp on a 50-query dogfood
    eval set per index + no regression on the warm-loop benchmark.

### #91 Global config strict-parse harmonization (PARTIAL) — CLOSED 2026-05-21 via story 5.22
- **Current state**: Curator scope closed in story 4.14; non-curator config families remain mixed.
- **Phase 5 expectation**: close with global strict parse/fail-fast policy.
- **Closed by**: story 5.22 lifted the strict-parse helpers
  (`parse_bool_strict`, `parse_{u32,u64,f32}_strict`, `env_*_or_default`) out of
  `crates/seasoned-hand-server/src/main.rs` into
  `crates/seasoned-hand-core/src/config/strict.rs` so server + CLI + every worker
  spawn share one implementation. The two non-curator typed flags that still used
  permissive parses (`SEASONED_HAND_ROLLBACK_ON_VERIFIER_FAIL`, `SH_LEARNING_ENABLED`)
  now go through `env_bool_or_default` and fail-fast at boot on invalid values.
  7 core unit tests + 5 server integration tests + 3 CLI integration tests pin
  the contract.

### #92 Adaptive auto-archive thresholds (H3)
- **Current state**: Static per-project thresholds shipped; adaptive policy deferred.
- **Phase 5 expectation**: close or explicitly defer with evidence-backed rationale.

### #93 Optional fork-promotion governance (H3)
- **Current state**: Revision-chain baseline shipped; optional governance mode deferred.
- **Phase 5 expectation**: decide optional governance path for shared org playbooks.
- **Closure (story 5.8)**: policy-surface baseline shipped. `playbook_shares.visibility_state`
  has three states (`review`, `shared`, `suspended`); `PlaybookShareService::curator_auto_share`
  routes high-confidence revisions to `shared`, low-confidence to `review`. Manual-publish-only
  governance ("never auto-share, require operator approval") is a runtime configuration of the
  same surface — set `archive_apply_min_confidence` above the maximum reachable confidence
  (e.g. `1.01`) and every curator-created share lands in `review`. No additional code path
  needed; this satisfies the architecture §6.2 OQ #6 Option B deferred-debt note. **CLOSED**.

### #94 Retrospective tiered model-by-size policy (H3)
- **Current state**: Single summarizer profile baseline shipped.
- **Phase 5 expectation**: close or defer with cost/quality threshold criteria.

### #96 Curator rationale schema evolution tooling (H3)
- **Current state**: Structured rationale exists; schema-evolution tooling deferred.
- **Phase 5 expectation**: add compatibility/versioning tooling baseline.

### #97 Per-crate dependency justification discipline (H3) — CLOSED 2026-05-21 via story 5.23
- **Current state**: Documentation discipline deferred.
- **Phase 5 expectation**: close by enforcing per-crate justification + ARCH §1 addendum updates for
  net-new dependencies.
- **Closed by**: story 5.23 added a Phase 5 dependency addendum block to
  `specs/01-architecture/ARCHITECTURE.md` §1 (Phase 5 introduced zero net-new
  workspace dependencies — multi-user / RBAC / audit / cost / redaction / OCC
  all built on the Phase 0-4 crate set). `scripts/spec-check.sh` Check #9
  enforces the existence of the addendum block as a discipline gate; any
  future story that adds a workspace dependency must extend the addendum
  with a per-crate justification or the gate fails.

## Additional carry-forward from cross-phase security review

### #S-1 Tenant-scoped event redaction policy unresolved (NEW carry-in) — CLOSED 2026-05-20 via story 5.14
- **Source**: `specs/SECURITY_REVIEW.md` iter-3 observation (2026-05-20).
- **Current state**: Action/Observation events may carry raw tool args/outputs.
- **Phase 5 expectation**: resolve via explicit tenant-visible redaction/access policy and tests.
- **Closed by**: story 5.14 (`crate::events::visibility` write-time redaction hook on every
  `SqliteEventStore::append`). Every event now gets a `tenant_event_view` row with PII patterns
  (PEM keys, IPv6, Authorization headers, etc.) stripped via `verifier::extraction::redact_pii`.
  Stories 5.15/5.16 layer RBAC predicates + admin raw-event route on top of the projection.

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
