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
