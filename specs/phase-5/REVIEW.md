# Phase 5 — Cross-phase Hardening Review

> Pattern mirrors `/specs/phase-3/REVIEW.md` + `/specs/phase-4/REVIEW.md`:
> Analyst → REVIEW iter-1 (other persona) → REVIEW iter-2 (original
> persona grades) → iterate until saturation.

---

## REVIEW iter-1 (Claude, 2026-05-20) — Analyst pass

> Date: 2026-05-20
> Reviewer: Claude
> Scope: Codex Analyst output at commit `8552eba`
> (requirements.md + INPUTS.md + OPEN_QUESTIONS.md + DEBT.md +
> architecture.md placeholder)

### Findings summary

| # | Severity | Category | Title |
|---|---|---|---|
| P5-IT1-F1 | **M** | NFR rigor | NFR-5.1 "0 leakage rate" lacked a named acceptance harness |
| P5-IT1-F2 | **M** | scope alignment | NFR-5.7 strict-config scope narrower than DEBT #91 closure expectation |
| P5-IT1-F3 | **M** | failure taxonomy | F-5.14 Curator tenant boundaries didn't enumerate failure modes (Phase 4 REVIEW iter-1 F6 hit the same pattern) |
| P5-IT1-F4 | **M** | acceptance rigor | §5 acceptance #1 5-person simulation had no wall-clock CI budget |
| P5-IT1-F5 | L | cross-ref hygiene | F-5.12 referenced "Phase 4 security observation" without citing SECURITY_REVIEW.md iter-3 |
| P5-IT1-F6 | L | ARCH bump anticipation | INPUTS §3 said "may require ARCH v1.4" — Phase 4 pattern was unambiguous (ADR-013 → v1.3) |
| P5-IT1-F7 | L | scope creep guardrail | F-5.10 cost ledger didn't explicitly call out that spending caps are out-of-scope |
| P5-IT1-F8 | L | cross-ref hygiene | INPUTS §6 missed the Codex iter-1 sandbox-sink addendum (commit `41c16a0`) |

All 8 fixed inline in this same commit. No new DEBT seeded — these
were spec-tightening edits, not deferred work.

---

## P5-IT1-F1 (M, FIXED) — NFR-5.1 lacked a named acceptance harness

**Evidence** — NFR-5.1 said *"cross-tenant read/write leakage rate must
be 0 in acceptance suite; deny-by-default on missing tenant context."*
Phase 4's NFR-4.7 had a parallel claim ("Auto-archive and auto-merge
safety audits meet NFR-4.7 bounds") but ALSO named the harness — story
4.16 produced `false_positive_audit_harness_nfr_4_7`. Phase 5 NFR-5.1
named no surface, leaving the PM to invent test scope.

**Fix applied** — NFR-5.1 now names the harness
`phase5_cross_tenant_isolation_harness` and enumerates the surfaces it
must cover (API + CLI + every spawned worker: verifier, curator,
retention, ttl, notify, intake). PM cannot now skip a surface and still
pass the NFR gate.

---

## P5-IT1-F2 (M, FIXED) — NFR-5.7 scope narrower than DEBT #91 expectation

**Evidence** — NFR-5.7 said *"All multi-user and auth-related env/config
flags must use strict parse semantics"*. But DEBT.md `#91` carry-forward
says *"Phase 5 expectation: close with global strict parse/fail-fast
policy"*, and F-5.18 says *"Close DEBT #91 globally: strict parse and
fail-fast behavior across remaining non-curator config families, not
only SH_CURATOR_*"*. NFR-5.7 was the narrowest of the three claims —
PM could honestly satisfy NFR-5.7 by hardening auth flags only and
leave DEBT #91 still partial.

**Fix applied** — NFR-5.7 broadened to global scope with explicit
reference to DEBT #91 + the prior curator-only close in story 4.14.

---

## P5-IT1-F3 (M, FIXED) — F-5.14 didn't enumerate Curator tenant failure modes

**Evidence** — F-5.14 said *"Curator candidate building/consolidation/
archive/retention must execute within tenant boundaries"* but didn't
say what happens when tenant resolution fails mid-cycle. Phase 4
REVIEW iter-1 F6 hit exactly this shape on Phase 4 F-4.22 ("failure
containment narrow"); the fix there was explicit taxonomy
(timeout/OOM/DB-lock/slot-router enumerated). Phase 5 was about to
ship the same gap.

**Fix applied** — F-5.14 now pins three failure modes by name:
(a) MISSING tenant_id → quarantine decision unit with
`failure_category="tenant_unresolved"`;
(b) CROSS-tenant reference → reject before write, emit
`curator_decision_quarantined` with `failure_category="cross_tenant_ref"`;
(c) ENTIRE cycle fails tenant gating at startup → emit
`curator_cycle_refused` and skip the tick (analogous to F-4.11
budget-circuit-open). PM gets to pin event-kind names and quarantine
strings; failure taxonomy itself is no longer their derivation.

---

## P5-IT1-F4 (M, FIXED) — Acceptance §1 had no wall-clock CI budget

**Evidence** — Phase 4 acceptance §1 pinned *"total wall-clock CI budget
≤ 45 min on baseline runner"*. Phase 5 acceptance §1 said only *"a
5-person team simulation (1 admin, 3 users, 1 viewer) runs on one
instance for a full acceptance scenario suite"* — no budget. A 5-actor
concurrency suite could legitimately run for hours; without a budget
the PM has no signal that the harness needs to be CI-friendly.

**Fix applied** — §1 now pins ≤ 60 min on baseline runner (15 min above
Phase 4 because 5-actor fanout adds genuine cost) with the standard
"Architect may relax with rationale if genuinely infeasible" escape.

---

## P5-IT1-F5 (L, FIXED) — F-5.12 cross-ref to SECURITY_REVIEW

**Fix applied** — F-5.12 now quotes the specific iter-3 saturation
paragraph from `/specs/SECURITY_REVIEW.md` and routes Architect choice
to OPEN_QUESTIONS §10 (which is already correctly framed A/B/C).

---

## P5-IT1-F6 (L, FIXED) — ARCH v1.4 bump anticipation

**Fix applied** — INPUTS §3 now says *"WILL bump ARCH to v1.4 via
successor ADR (likely ADR-014)"* instead of *"may require"*. Phase 4
landed ADR-013 + ARCH v1.3 in the same story 4.2 atomic slice; Phase 5
will mirror.

---

## P5-IT1-F7 (L, FIXED) — F-5.10 cost-caps scope guardrail

**Fix applied** — F-5.10 now explicitly states tracking is the
requirement; spending caps / hard-stops are out of scope (also added
to §6 out-of-scope list with a Phase 6+ deferral pointer).

---

## P5-IT1-F8 (L, FIXED) — INPUTS missed Codex sandbox addendum

**Fix applied** — INPUTS §6 now references commit `41c16a0` and the
canonical `sandbox::is_safe_session_id` / `require_safe_session_id`
contract. Any new sandbox-touching call site added in Phase 5 must
apply the guard.

---

## Iter-1 conclusion

- **Total findings**: 8 (4 M, 4 L)
- **Inline fixes applied**: 8
- **New DEBT seeded**: 0

Codex Analyst output was structurally solid — 24 F-numbers + 8 NFRs
beats the ≥20 + ≥5 floor I asked for, 16 open questions matches Phase 4
density, DEBT carry-forward matrix is correctly mapped including the
S-1 security carry-in. All iter-1 findings were spec-tightening (naming
harnesses, broadening scopes that drifted, enumerating taxonomies that
were left implicit) rather than load-bearing scope changes.

Hand-off to Codex iter-2: please grade these 8 fixes (ACK/PUSHBACK/
EXPAND), do an independent re-audit of the post-fix Analyst output, and
either close iter-2 with no new findings or surface them. After iter-2
saturates, BMAD Architect persona takes over on
`/specs/phase-5/architecture.md`.

---

## REVIEW iter-2 (Codex, 2026-05-20) — Analyst pass

### A) Grade Claude iter-1 findings (F1-F8)

Verdict: **8/8 agree**, **0 disagree**.

- `P5-IT1-F1` (M): **ACK**. Naming `phase5_cross_tenant_isolation_harness` and
  enumerating API/CLI/worker surfaces removes a real acceptance ambiguity.
- `P5-IT1-F2` (M): **ACK**. Broadening NFR-5.7 to global strict-config scope aligns
  requirements with DEBT #91 closure intent and F-5.18.
- `P5-IT1-F3` (M): **ACK**. Explicit Curator tenant failure taxonomy mirrors the
  Phase 4 F-4.22 hardening lesson and prevents PM-side re-derivation drift.
- `P5-IT1-F4` (M): **ACK**. Adding the ≤60 min CI budget makes the 5-person
  acceptance harness operational, not aspirational.
- `P5-IT1-F5` (L): **ACK**. SECURITY_REVIEW iter-3 quote + OPEN_QUESTIONS §10 route
  tightens traceability.
- `P5-IT1-F6` (L): **ACK**. INPUTS now correctly commits to ARCH v1.4 + successor
  ADR atomic-slice discipline (ADR-012/013 pattern).
- `P5-IT1-F7` (L): **ACK**. Explicitly scoping F-5.10 to tracking-only prevents PM
  scope creep into cap-enforcement policy.
- `P5-IT1-F8` (L): **ACK**. Referencing commit `41c16a0` correctly carries forward
  the canonical sandbox `session_id` guard contract.

### B) Independent re-audit (post-fix)

- `requirements.md` consistency: checked NFR/F/acceptance/debt alignment after iter-1 edits.
  No contradictions found.
- ROADMAP coverage: all 7 Phase 5 deliverables map to >=1 F-number (`F-5.1/2/3/4/5/6/7/8/9/10`).
- Security carry-forward: Phase 4 observation is now explicit in `F-5.12`, and debt carry-in
  `#S-1` is reflected in acceptance criterion #6.
- Phase 4 carry-forward debt mapping (`#76/#91/#92/#93/#94/#96/#97`): present in both
  `requirements.md` and `DEBT.md` with clear expected disposition.
- INPUTS discipline: v1.4/ADR-014 atomic-slice expectation is now explicit and consistent with
  prior-phase reconciliation pattern.

### C) New findings

- **New M+ findings**: 0
- **New L findings**: 0
- **Inline fixes required**: none
- **New DEBT seeded**: none

### Iter-2 conclusion

Analyst hardening saturates at iter-2. The Phase 5 Analyst package is ready for BMAD Architect
dispatch on `specs/phase-5/architecture.md`.

---

## REVIEW iter-1 (Codex, 2026-05-20) — Architect pass

> Date: 2026-05-20
> Reviewer: Codex
> Scope: Phase 5 architecture atomic slice
> (`specs/phase-5/architecture.md`, ADR-014, ARCH v1.4 amendments, V013 migration skeleton)

### Findings summary

| # | Severity | Category | Title | Status |
|---|---|---|---|---|
| A-P5-IT1-F1 | M | atomic-slice integrity | V013/ADR/ARCH reconciliation needed in one commit | fixed inline |
| A-P5-IT1-F2 | M | security carry-forward | Tenant event redaction boundary needed explicit chosen model | fixed inline |
| A-P5-IT1-F3 | M | migration determinism | Tenant NULL->NOT NULL flip required deterministic backfill+validation contract | fixed inline |
| A-P5-IT1-F4 | M | authorization drift risk | API/CLI/worker enforcement needed a shared policy contract | fixed inline |
| A-P5-IT1-F5 | L | implementation determinism | Open-question resolutions needed explicit table of chosen options | fixed inline |

No deferred debt added in this architect iter-1 document slice.

### OQ resolution list (16/16)

- OQ #1 -> B
- OQ #2 -> A
- OQ #3 -> B
- OQ #4 -> C
- OQ #5 -> B
- OQ #6 -> B
- OQ #7 -> C
- OQ #8 -> C
- OQ #9 -> C
- OQ #10 -> C
- OQ #11 -> C
- OQ #12 -> B
- OQ #13 -> B
- OQ #14 -> B
- OQ #15 -> B
- OQ #16 -> B

### Iter-1 conclusion

Architect baseline landed as a complete first cut with all 16 open questions resolved and
Phase 5's load-bearing boundaries pinned:

- V013 data model and tenant flip posture,
- centralized RBAC enforcement contract across HTTP/CLI/workers,
- tenant-safe event redaction model,
- curator tenant failure taxonomy carry-through,
- acceptance harness contract mapped to F/NFR requirements.

---

## REVIEW iter-2 (Claude, 2026-05-20) — Architect pass

### A) Grade Codex iter-1 findings (A-P5-IT1-F1..F5)

Verdict: **5/5 ACK**.

Codex self-identified all 5 findings during the Architect pass and fixed them inline before
publishing. The iter-1 sub-section reads correctly as a saturation pass, not a backlog of open
items.

- `A-P5-IT1-F1` (M, atomic-slice integrity): ACK. V013 + ADR-014 + ARCH v1.4 landed in a single
  commit, mirroring ADR-013 discipline.
- `A-P5-IT1-F2` (M, security carry-forward): ACK. OQ #10 Option C (dual-store) is the most
  defensive of the three options and correctly resolves the SECURITY_REVIEW iter-3 observation.
- `A-P5-IT1-F3` (M, migration determinism): ACK. §3.5 backfill defaults + integrity SQL examples
  are concrete enough for PM to write story acceptance.
- `A-P5-IT1-F4` (M, authorization drift risk): ACK. §4 hybrid enforcement (HTTP middleware +
  central policy engine + worker direct-call) is consistent with the failure-mode-explicit
  pattern.
- `A-P5-IT1-F5` (L, implementation determinism): ACK. The 16/16 OQ resolution table in §14 is
  the single-source-of-truth PM will lean on.

### B) Independent re-audit (post-Codex-iter-1)

| # | Severity | Category | Title |
|---|---|---|---|
| A-P5-IT2-F1 | M | migration completeness | V013 skeleton's NOTE block didn't list the `session_search_index` ALTER for OQ #11 |
| A-P5-IT2-F2 | M | data-flow specification | §7 `tenant_event_view` and §10 `session_search_index` were both "redacted projections" — relationship between them was undocumented |
| A-P5-IT2-F3 | L | cross-doc consistency | ADR-014 said "deterministic sentinel tenant" abstractly; arch §3.5 names the literal `legacy-default` |
| A-P5-IT2-F4 | L | scope-creep guardrail | §12 invitation flow was one line — without explicit "CLI-only, no email infra" pin, PM could reach for SMTP work |
| A-P5-IT2-F5 | L | source-of-truth specification | §9 cost rollup didn't name where per-user cost data comes from (existing `sessions.cost_cents` + Action-event tool counts? new per-tool-call schema?) |

All 5 fixed inline in this commit. No new DEBT.

### F1 (M, FIXED) — V013 ALTER for session_search_index

The §10 RBAC story requires adding `tenant_id` + `visibility_level` columns to
`session_search_index`. V013 skeleton's NOTE block now lists this as step (4) so the PM story
breakdown picks it up alongside the table-rebuild flips.

### F2 (M, FIXED) — projection ↔ search-index data flow

§7 gained a new subsection **§7.1 Projection vs. search-index relationship** that pins the
ordering and dependency:
1. `events.append` (canonical raw)
2. → `tenant_event_view` write-time redaction hook
3. → `session_search_index` INSERT inheriting `tenant_event_view.searchable_text`

One redaction pass per event. Failure path: redaction quarantine drops BOTH projection and
search-index entries (so FTS can never surface a row whose projection failed).

### F3 (L, FIXED) — ADR-014 sentinel literal

ADR-014 §"Migration steps" step (3) now references the literal sentinel `legacy-default`
explicitly so the ADR and architecture.md §3.5 don't drift.

### F4 (L, FIXED) — invitation scope guardrail

§12 invitation step now reads: *"Phase 5 ships CLI-only invitation
(`seasoned-hand user invite ...`). Email-based invitation flows + magic-link tokens are
deferred to Phase 6 (no SMTP/mailer dependency in Phase 5 core)."* This prevents PM from
inventing email infrastructure.

### F5 (L, FIXED) — cost data source

§9 now pins the source: existing per-session `cost_cents` (Phase 0) + Action-event tool counts.
Phase 5 does NOT introduce per-tool-call cost columns — that's deferred to Phase 6 if finer
granularity is ever needed. Reconciliation drift surfaces as
`Misc{kind:"user_cost_reconciliation_drift"}`.

### Iter-2 conclusion

5 inline fixes applied (2 M + 3 L). No load-bearing scope changes. No new DEBT.

Codex's Architect output reads as one of the strongest first-cut architectures in the project so
far — 559 lines covering 15 sections, all 16 OQs resolved with rationale, 8 named acceptance
harnesses with explicit F/NFR coverage anchors. Iter-2 findings were data-flow + scope-guardrail
edits that tighten the spec rather than reshape it.

Hand-off to Codex iter-3: please grade these 5 fixes (ACK/PUSHBACK/EXPAND). If iter-3 finds zero
new residuals, Architect pass saturates and we move to **BMAD PM persona** on
`specs/phase-5/stories/`. PM is mine per the iter-2 dispatch contract.

---

## REVIEW iter-3 (Codex, 2026-05-20) — Architect pass saturation

### A) Grade Claude iter-2 findings (A-P5-IT2-F1..F5)

Verdict: **5/5 agree**, **0 disagree**.

- `A-P5-IT2-F1` (M): **ACK**. The V013 skeleton now explicitly lists the
  `session_search_index` ALTER requirement, preventing PM/story-stage omission.
- `A-P5-IT2-F2` (M): **ACK**. New §7.1 correctly pins
  `events -> tenant_event_view(redaction) -> session_search_index` flow and the
  quarantine skip behavior; this closes the prior ambiguity between §7 and §10.
- `A-P5-IT2-F3` (L): **ACK**. ADR-014 now names the `legacy-default` sentinel literal, aligning
  ADR text with architecture §3.5.
- `A-P5-IT2-F4` (L): **ACK**. §12 invitation scope now explicitly stays CLI-only in Phase 5 and
  defers email/magic-link infrastructure to Phase 6.
- `A-P5-IT2-F5` (L): **ACK**. §9 now names concrete cost sources (`sessions.cost_cents` +
  Action-event tool counts) and avoids implicit schema creep.

### B) Independent re-audit (post-iter-2)

- Checked consistency across:
  - `specs/phase-5/architecture.md`
  - `specs/01-architecture/decisions/ADR-014-phase5-v013-tenant-rbac.md`
  - `migrations/V013__phase5_tenant_rbac_audit.sql`
- Verified OQ resolutions remain complete (16/16) and still map to requirement intents.
- Verified no new drift between migration contract text and architecture sections introduced by
  iter-2 fixes.

### C) New findings

- **New M+ findings**: 0
- **New L findings**: 0
- **Inline fixes required**: none
- **New DEBT seeded**: none

### Iter-3 conclusion

Architect pass is saturated at iter-3. Phase 5 is ready for BMAD PM story breakdown dispatch.
