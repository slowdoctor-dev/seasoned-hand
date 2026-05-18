# Phase 3 — Technical Debt Ledger

> Append-only list of shortcuts, stubs, simplifications, and deferred
> work introduced during Phase 3. Same discipline as Phase 0 / Phase 1
> / Phase 2 DEBT.md.
>
> **Status (pre-kickoff, 2026-05-16)**: empty header. Phase 3 has not
> yet been seeded by the BMAD Architect pass on
> `/specs/phase-3/architecture.md`. Seed items will land here once
> the architecture doc is written and reviewed.
>
> Pay-down inheritance from earlier phases (informational only — do
> NOT duplicate entries here, see the source ledgers):
>
> - Phase 2 DEBT #5 — Provenance manifest size budget (Curator may
>   compress old manifests at Phase 3+)
> - Phase 2 DEBT #6 — Skill / playbook tables empty (Phase 3 fills them)
> - Phase 2 DEBT #7 — Verifier rollback default still opt-in
>   (re-evaluate with Phase 3 verdict data)
> - Phase 2 DEBT #28 — Replay cost baseline resets to zero on rebuild
>   (Phase 3 may emit periodic `cost_snapshot` Misc events)
> - Phase 2 DEBT #31 (rough edge 3) — BriefingCard server-side
>   validation error UX
> - Phase 2 DEBT #52 — `lib.rs` 2879-line split (Phase 3 warm-up)
> - Phase 2 DEBT #58 (remainder) — `pub` → `pub(crate)` shrinkage
> - Phase 2 DEBT #60 — Phase 1 large-file split set
> - Phase 2 DEBT #61 — `EventType::Knowledge/Datasource/Skill` emit
>   wiring (Phase 3 Curator)
> - Phase 2 DEBT #62 — `spec-check.sh` phase-version gate
> - Phase 2 DEBT #63 — Frontend `pnpm test` stub
> - Phase 1 DEBT #3 — Verifier rollback default (see Phase 2 DEBT #7)
>
> Closed by the Codex follow-up sequence (2026-05-17) before Phase 3
> kickoff (informational — these no longer carry to Phase 3):
>
> - Phase 2 DEBT #65 — Phase 1 verifier + checkpoint route loopback
> - Phase 2 DEBT #66 — /ws loopback gate
> - Phase 2 DEBT #67 — checkpoint rollback state coherence
> - Phase 2 DEBT #68 — plan phase/title caps
> - Phase 2 DEBT #69 — loopback regression test sweep
> - Phase 2 DEBT #70 — channel introspection routes loopback
> - Phase 2 DEBT #71 — Track C screenshot byte cap

---

## Seed (TBD)

_To be populated by the BMAD Architect pass on
`/specs/phase-3/architecture.md`._

---

## Categories quick-reference (same as Phase 0 / Phase 1 / Phase 2)

| Severity | Meaning |
|---|---|
| **H** | Blocks the next phase's goals if not addressed |
| **M** | Will bite at scale or in a year, manageable today |
| **L** | Documentation / minor friction / one-line fix later |

## Seed (from BMAD Architect pass, 2026-05-17)

1. **#72 (M)** Embedding rerank deferred
- **What**: Production matcher remains FTS5-only in Phase 3; embedding-based rerank is not shipped.
- **Why now**: F-3.5 pins FTS5 production matcher; embedding slot wiring is Phase 4+ scope.
- **Pay down**: Phase 4 Curator, after telemetry baseline from `playbook_match` events.

2. **#73 (M)** Knowledge/Datasource writers deferred
- **What**: `EventType::Knowledge` and `EventType::Datasource` remain reserved-but-unwired.
- **Why now**: F-3.16 requires index coverage, not active writers; L2 cross-source enforcement is deferred.
- **Pay down**: Phase 4 with L2 rollout and concrete emit semantics.

3. **#74 (M)** Tenant semantics deferred
- **What**: Phase 3 matching is project-scoped only; tenant/global promotion policy is not implemented.
- **Why now**: Requirements F-3.12/§5 defer full tenant isolation semantics to Phase 5.
- **Pay down**: Phase 5 multi-user policy + migration plan.

4. **#75 (L)** Search summarizer degradation telemetry hardening
- **What**: Session-search summary path degrades to raw hits on LLM failure; richer retry/backoff policy not included.
- **Why now**: Keeps Phase 3 deterministic and non-blocking while preserving operability.
- **Pay down**: Phase 4 operations hardening pass.

5. **#76 (L)** FTS5 production-matcher weights are seed values
- **What**: Architecture §11 pins `5.0*kw_hits + 3.0*title_hits + 1.0*content_hits`
  plus a 60-day linear recency decay as initial scoring weights. None are derived from
  dogfood data.
- **Why now**: Phase 3 has no telemetry to tune from; seed values must be pinned for
  deterministic CI.
- **Pay down**: Phase 4 Curator retunes from `Skill{kind:"match"}` telemetry once a
  match corpus exists.

6. **#77 (M)** Project-scope JOIN may need denormalization
- **What**: Matcher enforces F-3.12 via `playbooks JOIN tasks ON playbooks.source_task_id`
  + `WHERE tasks.project_id = :new_task.project_id`. At Phase 3 scale (<=10k
  playbooks per matcher query budget in §7) this fits the 80 ms p95 budget; at Phase 4+
  scale the JOIN may dominate.
- **Why now**: V010 deliberately does NOT add a denormalized `source_project_id` column
  to keep the migration minimal and avoid a second ARCH §2.5 divergence.
- **Pay down**: Phase 4 perf pass — if `Skill{kind:"match"}` p95 latency telemetry
  exceeds 80 ms, add `source_project_id TEXT` column + index + backfill in V011.

7. **#78 (H)** ADR-012 required when V010 ships
- **What**: Architecture §10.3 requires successor ADR-012 in the SAME PR slice as V010
  per F-3.19 atomic-slice rule. The ADR documents content_path-as-reserved,
  ALTER-TABLE schema shape, and the FTS5 maintenance triggers — none currently in
  ARCH §2.5 v1.1.
- **Why now**: Phase 3 cannot land V010 without the spec reconciliation per F-3.19;
  this is a HARD dependency, not a "later cleanup".
- **Pay down**: First Phase 3 story that touches V010 must include ADR-012 +
  ARCH §2.5 v1.1 → v1.2 bump in the same PR.

8. **#79 (M)** ADR-013 forward at Phase 5 multi-user kickoff
- **What**: V010 ships `sops` and `glossary` WITHOUT `tenant_id` columns. Phase 5
  multi-user will need to add tenant_id (nullable then NOT NULL pattern, same as V009
  did for `playbooks` and `skills`).
- **Why now**: Phase 3 is single-operator; adding unused columns now would violate
  the "no speculative code" principle.
- **Pay down**: First Phase 5 story authors ADR-013 + schema migration adding
  `tenant_id` to `sops` and `glossary` with backfill plan.

## Phase 3 close-out (story 3.16)

- **Closed inherited debt**: Phase 2 DEBT #62 (`spec-check.sh` phase-version gate).
  - Evidence:
    - `scripts/spec-check.sh` now enforces Phase 3 hook markers:
      - V010 migration presence and `sops`/`glossary` table definitions
      - ARCH version `v1.2` reconciliation marker
      - Phase 3 required CLI groups in `specs/phase-3/architecture.md`
  - Landed with Story 3.16 acceptance gate slice.

## REVIEW iter-1 (Claude, 2026-05-18) — DEBT additions

9. **#80 (M)** Gate-matcher fixture/brief sentinel coupling
- **What**: `matcher::gate_match` (`crates/seasoned-hand-core/src/matcher/mod.rs:88-96`)
  encodes the F-3.4 identity tuple `(fixture_id, normalized_brief)` as TWO LIKE-substring
  matches on `playbooks.trigger_keywords`. The seed at `verifier/gate.rs:1334` writes
  `["fixture:<id>", "brief:<normalized>"]` into the JSON-shaped trigger_keywords.
- **Why it's debt**: (a) FTS5 `playbooks_fts` indexes trigger_keywords, so production-mode
  queries can hit gate-fixture sentinels; (b) SQL LIKE substring match allows prefix-
  collision false positives (`fixture:phase2_overnight_default_path` would match
  `fixture:phase2_overnight_default_path_v2`); (c) the tuple is encoded structurally
  (JSON-array text) rather than as a typed schema, so Phase 4 retuning of trigger_keywords
  semantics could silently break gate matching.
- **Why now**: Phase 3 ships green tests with the substring approach. Refactoring to
  dedicated columns or a `gate_fixtures` join table is a deeper schema change.
- **Pay down**: Phase 4 perf/quality pass — add `fixture_id TEXT` + `gate_brief_hash TEXT`
  columns to `playbooks` (or a separate `gate_fixtures` table) + regression test
  ("seed a benign playbook whose content contains 'fixture:' and assert production mode
  does NOT return it under a benchmark-shaped query").

10. **#81 (M)** Extraction PII regex over-redaction (PHONE_RE, IPV4_RE)
- **What**: `verifier/extraction.rs:24-29` `PHONE_RE` matches any 2-16 digit run starting
  with non-zero (false positives: order IDs, version numbers, timestamps). `IPV4_RE`
  matches `\d{1,3}.\d{1,3}.\d{1,3}.\d{1,3}` — also matches software version strings
  like `1.2.3.4` and build IDs like `2025.10.15.0`.
- **Why it's debt**: extracted playbook content with technical version references gets
  garbled into `[REDACTED_IP]` and `[REDACTED_PHONE]`, degrading signal density inside
  the NFR-3.3 12 KB injection budget.
- **Why now**: F-3.14 was authored as a security floor; over-redaction is the safer
  failure mode for Phase 3. Tightening regex risks under-redaction without a corpus
  to validate against.
- **Pay down**: Phase 4 Curator builds a test corpus capturing the false-positive
  cases (version strings, build IDs, timestamps) and tightens `PHONE_RE` (require
  `+` prefix OR separator) + `IPV4_RE` (skip when preceded by `v`/`version` or
  any octet > 255 making it not a valid IP).

11. **#82 (L)** No orchestrator-level test for combined "post-cap quality_floor fail" emit
- **What**: Architecture §3 step 6 mandates: when NFR-3.5 output cap fires AND post-cap
  content drops below the F-3.18 quality floor, BOTH `playbook_extraction_output_capped`
  AND `playbook_extraction_rejected{layer:"quality_floor"}` events must emit. Unit
  tests at `verifier/extraction.rs:294-298` test the helpers; no integration test
  drives a real extractor through the combined branch.
- **Why it's debt**: orchestrator could regress to emitting only one event after a
  refactor; nothing catches it.
- **Pay down**: integration test under the extraction orchestrator's test module
  that synthesizes a 24KB+ LLM response with step content that loses its 200-char
  floor after capping; asserts both Misc events appear in the events table for the
  synthetic session.

12. **#83 (process)** AGENTS.md §6 full gate list at phase close-out
- **What**: Story 3.16 acceptance verification ran only the Phase-3-specific tests +
  `spec-check.sh` — skipped `cargo test --workspace`, `cargo clippy --all-targets
  -- -D warnings`, `cargo fmt --check`. This let F1/F2/F3 ship to main.
- **Pay down (already applied)**: REVIEW iter-1 backfilled story 3.16's Verification
  section to include the full AGENTS.md §6 gate list. Future phase close-out stories
  (Phase 4+) should template from story 3.16's updated section.
