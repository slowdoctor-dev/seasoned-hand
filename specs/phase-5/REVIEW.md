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
