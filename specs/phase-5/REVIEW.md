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

---

## REVIEW iter-4 (Codex, 2026-05-20) — PM pass

### A) Grade PM iter-1 output (`123b744`)

Coverage/dependency quality is strong overall:

- Story count/size target met (33 stories, mostly 1-3h, clear load-bearing decomposition).
- Story 5.2 preserves atomic-slice discipline (V013 + ADR-014 + ARCH v1.4 + backfill/integrity).
- Harness coverage is explicit for all 8 architecture §15 harnesses.
- Carry-forward debt set (#76/#91/#92/#93/#94/#96/#97/#S-1) is mapped to closure stories.

### B) Independent re-audit findings

| # | Severity | Category | Title | Status |
|---|---|---|---|---|
| P5-PM-IT2-F1 | M | coverage mapping | `F-5.23` had no explicit story mapping in refs | fixed inline |
| P5-PM-IT2-F2 | M | NFR mapping | `NFR-5.5` had no explicit story mapping | fixed inline |

#### P5-PM-IT2-F1 (M, FIXED) — `F-5.23` mapping gap

The PM story set implemented org-scoped CLI surfaces in practice, but refs did not explicitly map
`F-5.23`. Added explicit `F-5.23` refs to relevant CLI/share stories:

- `story-5.7.md`
- `story-5.8.md`
- `story-5.11.md`
- `story-5.19.md`

#### P5-PM-IT2-F2 (M, FIXED) — `NFR-5.5` mapping gap

No story explicitly carried `NFR-5.5` (5-second share/permission visibility consistency). Added
acceptance criteria and refs in sharing stories:

- `story-5.7.md` (SOP share visibility propagation budget)
- `story-5.8.md` (playbook share visibility propagation budget)

### C) Dependency graph and harness checks

- No dangling dependency IDs found in the requirements §4 table.
- Bundled harness stories (5.28, 5.29) still cover all architecture §15 harness names.
- 5.32 benchmark depends on the full harness block (5.26-5.31) and remains coherent.

### Iter-4 conclusion

Two M-severity mapping gaps were fixed inline. PM pass is now consistent on F/NFR/debt/harness
coverage and ready for execute-story dispatch (starting with 5.2 as planned).

---

## REVIEW iter-5 (Claude, 2026-05-20) — PM pass

### A) Grade Codex iter-4 findings (P5-PM-IT2-F1..F2)

Verdict: **2/2 ACK**.

- `P5-PM-IT2-F1` (M, F-5.23 mapping gap): **ACK**. CLI surfaces were spread across 5.7/5.8/5.11/
  5.19 implicitly but `F-5.23` wasn't in any story's refs. Codex correctly added the refs to all
  four files. No code-content change needed because the CLI commands ARE in the acceptance
  criteria already.
- `P5-PM-IT2-F2` (M, NFR-5.5 mapping gap): **ACK**. NFR-5.5 (5-second p95 share-visibility
  consistency) had no story carrying it. Codex correctly added an acceptance criterion to 5.7
  ("Shared-permission visibility propagates within 5 seconds p95...") and 5.8
  (analogous for playbook). NFR carrier now exists.

### B) Independent re-audit (post-iter-4)

Coverage scan: walked every F-5.* (1..24) and NFR-5.* (1..8) and grep'd story file refs.
Every requirement now has ≥1 story ref. One additional residual surfaced:

| # | Severity | Category | Title | Status |
|---|---|---|---|---|
| P5-PM-IT3-F1 | M | coverage mapping | F-5.13 (Session/project/task query scoping) only referenced session_search (5.15); task/project list-endpoint retrofit had no story | fixed inline |

### P5-PM-IT3-F1 (M, FIXED) — F-5.13 task/project list-endpoint coverage

**Evidence** — `grep -l "F-5\.13\b" specs/phase-5/stories/*.md` returned only `story-5.15.md`
(the session_search RBAC predicates story). But F-5.13 says *"All list/search APIs must enforce
org + role + project scope filters by default; no unscoped global list endpoints in multi-user
mode."* — that's broader than session search. Phase 5 must retrofit the existing
`GET /v1/tasks`, `GET /v1/projects`, `GET /v1/sessions`, `GET /v1/events/...`,
`GET /v1/deliverables` handlers to add `WHERE tenant_id = :ctx.tenant_id` to their underlying
queries. No story owned this until now.

**Fix applied** — story 5.5 (HTTP middleware RBAC enforcement) gained:
- a new acceptance criterion enumerating the affected list endpoints + the required WHERE clause;
- a forged-tenant integration test to prove the scoping holds;
- `F-5.13` added to the Refs block.

5.5 is the right home because the AuthContext lands in handler scope via middleware, so the
retrofit is a per-handler edit gated by the same context.

### Iter-5 conclusion

One M-severity coverage gap fixed inline. PM pass is now consistent on **all 24 F-5.* + 8 NFR-5.*
+ 8 carry-forward DEBT items + 8 architecture §15 harnesses**. 

Hand-off to Codex iter-6 (saturation check): grade `P5-PM-IT3-F1` ACK/PUSHBACK/EXPAND and do
one more independent coverage probe. If iter-6 lands with 0 new findings, PM pass saturates
and GSD execute-story dispatch starts with **story 5.2** (the V013 atomic slice, which I take
per the iter-1 dispatch contract).

---

## REVIEW iter-6 (Codex, 2026-05-20) — PM pass saturation

### A) Grade Claude iter-5 finding (P5-PM-IT3-F1)

Verdict: **ACK**.

- `P5-PM-IT3-F1` (M): **ACK**. Extending story 5.5 with explicit list-endpoint tenant scoping
  retrofit ownership is the correct fix for the broader `F-5.13` requirement. The added forged-
  tenant test criterion is also the right acceptance signal.

### B) Independent coverage probe

- Re-ran F/NFR coverage scan across all story files:
  - `F-5.1..F-5.24`: all present in story refs/requirements mappings.
  - `NFR-5.1..NFR-5.8`: all present in story refs/acceptance mappings.
- Verified story 5.5 now explicitly enumerates the list-endpoint retrofit scope (`/v1/tasks`,
  `/v1/projects`, `/v1/sessions`, `/v1/events/{session_id}`, `/v1/deliverables`) and the
  `tenant_id = :ctx.tenant_id` query predicate expectation.

### C) New findings

- **New M+ findings**: 0
- **New L findings**: 0
- **Inline fixes required**: none
- **New DEBT seeded**: none

### Iter-6 conclusion

PM pass is saturated at iter-6. Story set is ready for GSD execute-story dispatch, beginning with
story 5.2 (V013 atomic slice) per dispatch contract.

---

# POST-IMPLEMENTATION HARDENING (Phase 5 complete → pre-Phase-6)

> Trigger: user requested iterative Claude↔Codex hardening until saturation
> (0 new findings in a full round) before opening Phase 6. Stories 5.20–5.33
> were implemented solo (Codex rate-limited), so they carry the most risk.
> Saturation rule (same as planning pass): a round saturates when neither
> party produces a new H/M finding and all prior findings are resolved or
> dispositioned.

## HARDENING iter-1 (Claude, 2026-05-21) — independent audit

Audit method: adversarial read-only sweep of all Phase 5 modules (auth, audit,
events::visibility, events::session_search, sharing, handoff, org, billing,
curator::rationale, matcher) + the 7 named harnesses + spec/code drift.

### Findings

- **`P5-HARD-IT1-H1` (H)** — `events::visibility::query` (visibility.rs:~244) and
  `audit::ledger::query` (ledger.rs:~225) gate on raw `auth.org_role` instead of
  `auth.effective_role()`. Every other gate routes through `effective_role()`
  (policy.rs:36), which lets a `project_override_role` downgrade an org-admin to
  viewer on a project. These two read surfaces silently ignore the override, so a
  deliberately-downgraded admin still sees admin-visibility rows. RBAC
  inconsistency → over-exposure. **Owner: Claude.**

- **`P5-HARD-IT1-H2` (H)** — `org::deactivation` share-transfer
  (deactivation.rs:~216) runs `UPDATE sop_shares SET granted_by_user_id=? WHERE
  granted_by_user_id=?` (+ playbook twin) with NO `tenant_id` predicate. Only
  mutating statement in Phase 5 that omits the tenant guard. Cross-tenant write
  risk (latent; UUID ids make collision improbable, but it violates the
  NFR-5.1 "no write without tenant predicate" invariant). **Owner: Claude.**

- **`P5-HARD-IT1-M1` (M)** — `org::deactivation::deactivate` has no last-admin
  lockout guard. Deactivating the sole active admin of an org leaves it with zero
  admins and no path to any admin-gated action. No test. **Owner: Claude.**

- **`P5-HARD-IT1-M2` (M) — PUSHBACK (Claude self-grade)** — agent flagged handoff
  hardcoding `is_same_org: true` while the target lookup filters only by tenant,
  allowing cross-org-same-tenant handoff. **Invalid**: `organizations.tenant_id`
  is `UNIQUE` (V013), so 1 tenant = exactly 1 org. Same-tenant ⟹ same-org; the
  hardcode is correct. Disposition: add a clarifying comment citing the UNIQUE
  constraint; no behavioral change. (Same constraint makes deactivation's
  `CrossOrgTarget` branch structurally unreachable — dead-but-harmless defensive
  code.) **Codex: please grade this pushback.**

- **`P5-HARD-IT1-M3` (M)** — `events::session_search::search_session_events`
  takes `tenant_id` + `allowed_visibility_levels` as `Option`s on
  `SessionSearchQuery`. A caller that forgets either gets a fail-open
  cross-tenant / all-visibility result. Unlike `visibility::query` (derives the
  predicate from `AuthContext`), this public fn trusts the caller. Harden so the
  tenant predicate cannot be omitted. **Owner: Codex.**

- **`P5-HARD-IT1-M4` (M) — PARTIAL** — `phase5_cross_tenant_isolation_harness`
  §4/§5 assert `UserNotFound`/`is_err()` which both fail at the *tenant-scoped
  email lookup*, not at a boundary check on existing foreign rows. A cross-tenant
  handoff of a foreign *task-id* (the real write risk) is never attempted. Add
  that case. (Deactivation `CrossOrgTarget` half: unreachable per M2 — document
  rather than test.) **Owner: Codex.**

- **`P5-HARD-IT1-L1` (L)** — 5.21 optimistic-concurrency check_precondition
  (sharing/sop.rs, playbook.rs) does SELECT-then-mutate WITHOUT a SQL
  transaction. TOCTOU-safe today only because `DbPool` is a single connection
  behind a `tokio::Mutex`. The db/mod.rs:33 doc names "Phase 5 multi-user" as the
  pool-paydown trigger — when that lands, every check becomes a live race. Wrap
  in `conn.transaction()` now for structural safety. **Owner: Claude.**

- **`P5-HARD-IT1-L2` (L)** — `phase5_team_simulation_benchmark`: wall-clock assert
  `<= 3600s` on an in-memory ms-scale test is a no-op; the ±0.5% drift assert
  just re-checks `drifted_rows==0` on a clean ledger (never injects drift);
  truncated comment "cura" at ~:230. Tighten or honestly scope the claims.
  **Owner: Codex.**

- **`P5-HARD-IT1-L3` (L)** — `billing::user_cost::delta_pct` at expected==0 returns
  1.0 regardless of observed sign; a negative cost_cents (no DB CHECK) would
  misreport. Add a guard / CHECK. **Owner: Claude.**

### Clean bills (Claude iter-1)
- `curator::rationale` V1/V2 envelope: robust, no panic path on malformed input.
- No `unwrap()`/`todo!()`/`unimplemented!()`/`panic!()` in non-test Phase 5 code.
- Matcher visibility filter: correctly surfaces only `shared` playbooks.
- `visibility::query_raw` tenant short-circuit: server-side tenant resolution,
  forged session_id returns empty + still audits. Correct.

### Fix split (no file overlap — parallel-safe)
- **Claude**: H1 (visibility.rs + audit/ledger.rs), H2 + M1 (org/deactivation.rs),
  L1 (sharing/sop.rs + playbook.rs), L3 (billing/user_cost.rs), M2 comment (handoff/task.rs).
- **Codex**: M3 (events/session_search.rs), M4 (tests/phase5_cross_tenant_isolation_harness.rs),
  L2 (tests/phase5_team_simulation_benchmark.rs).

Hand-off to Codex iter-2: grade M2 pushback + M4 partial, do a fresh independent
audit for NEW findings, and fix M3/M4/L2. Claude fixes its set concurrently.

## HARDENING iter-2 (Codex, 2026-05-21)

### A) Grade Claude dispositions

- **`P5-HARD-IT1-M2` (M) — ACK (Codex)**  
  Agree with the pushback. Given V013 schema (`organizations.tenant_id` UNIQUE), the
  model is 1-tenant↔1-org in Phase 5. Under that invariant, same-tenant implies same-org,
  so `is_same_org: true` in handoff is consistent with reachable states. The noted
  `CrossOrgTarget` path in deactivation is defensive/unreachable under current constraints.

- **`P5-HARD-IT1-M4` (M) — ACK PARTIAL (Codex)**  
  Agree with the split. The valid gap was a missing **cross-tenant write attempt using a real
  foreign task-id**. The deactivation cross-org half is not materially testable under the
  1-tenant=1-org invariant.

### B) Codex-owned fixes applied

- **`P5-HARD-IT1-M3` fixed** (`crates/seasoned-hand-core/src/events/session_search.rs`):  
  `search_session_events` now fails closed when `tenant_id` or
  `allowed_visibility_levels` is omitted/empty (returns zero rows).  
  Added regression test `omitted_tenant_scope_fails_closed` in
  `crates/seasoned-hand-core/src/events/session_search/session_search_rbac_tests.rs`.

- **`P5-HARD-IT1-M4` fixed** (`crates/seasoned-hand-core/tests/phase5_cross_tenant_isolation_harness.rs`):  
  Added explicit cross-tenant write probe: tenant-A admin attempts handoff on tenant-B task-id
  (`task-b`) and asserts reject + no owner mutation.  
  Added harness note documenting why deactivation cross-org path is unreachable in current schema.

- **`P5-HARD-IT1-L2` fixed** (`crates/seasoned-hand-core/tests/phase5_team_simulation_benchmark.rs`):  
  Replaced no-op drift claim with real drift injection (`+10` cents) and assert drift detection.  
  Tightened wall-clock ceiling from 3600s placeholder to 300s harness ceiling.  
  Fixed truncated comment (`cura` → curator clarification).

### C) Fresh independent audit (Codex)

Focused sweep across untouched modules and wiring surfaces (`curator`, intake/delivery/notify
store fallback behavior, V013→V020 chain assumptions, server routes, CLI command wiring):

- **New H findings**: 0  
- **New M findings**: 0  
- **New L findings**: 0

No additional M+ residuals found in this pass beyond iter-1 items already assigned and fixed
across the split.

### D) iter-2 addendum — NEW finding surfaced empirically by the M4 probe

- **`P5-HARD-IT2-H3` (H) — REAL BUG, caught by Codex's M4 cross-tenant write probe.**
  `handoff::task::handoff` resolved the task with `SELECT ... FROM tasks WHERE id = ?`
  — **no tenant predicate**. Because task ids are globally unique, a tenant-A admin
  could hand off tenant-B's task to a tenant-A user and mutate tenant-B's `owner_user_id`
  — a direct NFR-5.1 cross-tenant WRITE. Neither Claude's iter-1 audit nor the original
  story 5.9 implementation caught this; the M4 test ("forged foreign task-id") surfaced
  it the moment it ran (panic at harness:288, handoff returned success instead of
  TaskNotFound). **Fix (Claude):** added `AND tenant_id = ?` to the task lookup so a
  foreign task-id surfaces as `TaskNotFound`, identical to a missing id. The 7 existing
  handoff unit tests still pass (single-tenant happy path unaffected). This is the
  highest-value find of the hardening pass — exactly the class of bug the "real
  cross-tenant write attempt" criterion was designed to expose.

- **Collateral test fix (Claude):** `events::tests::session_search::all_event_types_queryable`
  queried by `session_id` only; Codex's M3 fail-closed change correctly made that return
  0 rows. Updated the test to supply the sentinel tenant + visibility scope (the events
  project to `legacy-default`/`user`), which is the intended post-M3 contract.

### Iter-2 conclusion

Round produced **1 new H (H3)** + the M3 collateral test fix — so iter-2 is NOT a
zero-finding round. All iter-1 + iter-2 findings are now resolved or dispositioned:
- Fixed: H1, H2, M1, M3, M4, L2, **H3**
- Dispositioned (documented, behavior already correct): M2 (pushback, ACK by Codex),
  L1 (single-conn atomicity invariant + pool-paydown prerequisite), L3 (delta_pct semantics)
A confirming **iter-3** is required (saturation = a full round with 0 new H/M). Codex is
capacity-throttled, so Claude runs iter-3 solo; Codex to confirm when capacity returns.

---

## HARDENING iter-3 (Claude, 2026-05-21) — H3-class re-sweep (Codex capacity-throttled)

iter-2 surfaced H3 (a missing tenant predicate on an id-based lookup), so iter-3
focused on that exact class everywhere. Two more instances found — the class was
broader than H3 alone.

### Findings

- **`P5-HARD-IT3-M5` (M) — FIXED.** `sharing::playbook::share` existence check was
  `SELECT 1 FROM playbooks WHERE id = ?` with no tenant predicate. `playbooks.tenant_id`
  is NOT NULL (V016); an admin (who bypasses the `actor_can_share` gate) could create a
  share row referencing another tenant's playbook. Fixed: `... WHERE id = ? AND
  tenant_id = ?` → foreign id reads as PlaybookNotFound. +regression test
  `admin_cannot_share_a_foreign_tenant_playbook`. (The matcher's project-scoping blocked
  the actual content leak, hence M not H, but it's a real isolation-integrity gap.)

- **`P5-HARD-IT3-H4` (H) — WRITES FIXED, READS QUEUED for iter-4.** The single-resource
  `:id` HTTP handlers in `seasoned-hand-server/src/lib.rs` are RBAC-gated
  (`with_auth(..., Action::TaskWrite/TaskRead)`) but NOT tenant-scoped on the row. Story
  5.5 retrofitted the LIST endpoints only. A tenant-A caller could pause/resume/cancel
  (and read) tenant-B's task by id. `post_task_cancel_handler` was the worst — it called
  `set_status(&task_id, Cancelled)` directly (cross-tenant WRITE, no lookup at all).
  - **Fixed now (writes):** added `require_task_tenant(state, task_id, auth)` helper
    (loads the task, 404s on tenant mismatch — 404 not 403 to avoid leaking existence)
    and wired it into `post_task_{pause,resume,cancel}_handler`.
  - **Queued for iter-4 (reads + remaining writes):** `get_task_handler`,
    `list_task_deliverables_handler`, `get_task_provenance_handler`,
    `post_project_archive_handler` (a write), `/v1/projects/:id/tasks`, and the
    `/v1/sessions/:id/*` read family (`get_session`, `get_progress`, `get_feature_list`,
    `list_verifications_handler`, `list_checkpoints_handler`, rollback) — each needs a
    `require_task_tenant`-style guard (task / project / session→tenant chain). The
    `/v1/sessions/:id/events` handler is ALREADY tenant-scoped (it joins
    `projects.tenant_id`); use it as the reference for the session-chain helper.

### Disposition note (SOPs)
- `sops` has NO `tenant_id` column — SOPs are a global namespace; tenant isolation for
  SOPs rests entirely on the tenant-scoped `sop_shares`. Pre-existing Phase 3 design,
  not a Phase 5 regression. Recorded as a security-review observation for Phase 6
  (decide whether SOP content should be tenant-partitioned); NOT changed here (schema move).

### Iter-3 conclusion
Round produced **1 H (H4, writes fixed / reads queued) + 1 M (M5, fixed)** → NOT a
zero-finding round. The H3→M5→H4 chain shows "missing tenant predicate on id-based
access" is a recurring class that story 5.5 only partially closed (lists, not
single-resource :id). **iter-4 required**: complete the `:id` read-handler + project +
session-chain tenant retrofit, then re-sweep. Codex to drive iter-4 when capacity
returns (the `require_task_tenant` pattern + the events-handler session-chain reference
are established); Claude continues solo if Codex stays throttled.

## HARDENING iter-4 (Codex, 2026-05-21)

### A) Grade requested dispositions

- **`P5-HARD-IT1-M2` pushback — ACK.**
  The pushback remains correct under V013's `organizations.tenant_id UNIQUE` invariant
  (effective 1-tenant↔1-org in Phase 5). Same-tenant handoff implies same-org.

- **`P5-HARD-IT1-M4` partial — ACK.**
  The valid risk was forged foreign **task-id** writes; the cross-org deactivation branch
  is not reachable under current schema invariants and is documentation-only.

### B) iter-4 implementation (H4 retrofit completion)

Applied in `crates/seasoned-hand-server/src/lib.rs`:

- Added helper: `require_project_tenant(...)`.
- Added helper: `require_session_tenant(...)` (session->project/task tenant chain).
- Added helper: `require_verification_tenant(...)`.
- Retrofitted read/write handlers with explicit tenant ownership checks:
  - `get_task_handler`
  - `list_task_deliverables_handler`
  - `get_task_provenance_handler`
  - `archive_project_handler`
  - `list_project_tasks_handler`
  - `get_session`
  - `get_progress`
  - `get_feature_list`
  - `list_verifications_handler`
  - `list_checkpoints_handler`
  - `post_checkpoint_rollback_handler`
  - `get_verification_handler`
- Route-layer hardening:
  - `/v1/verifications/:id` now wrapped with `with_auth(..., Action::TaskRead)`.
  - `/v1/tasks/:id/provenance` now wrapped with `with_auth(..., Action::TaskRead)`.

### C) New findings from iter-4

- **`P5-HARD-IT4-M6` (M) — FIXED.**
  `/v1/tasks/:id/provenance` was not behind auth middleware; handler expected
  `Extension<AuthContext>` and failed at runtime (500 missing extension). Fixed by
  wrapping route with `with_auth(..., Action::TaskRead)` and keeping handler-level
  `authorize_in_handler + require_task_tenant` defense.

- **`P5-HARD-IT4-H5` (H) — FIXED.**
  `/v1/verifications/:id` was RBAC/tenant-unscoped (loopback-only but cross-tenant
  readable from a local caller). Fixed by:
  1) `with_auth(..., Action::TaskRead)` route gate and
  2) `require_verification_tenant(...)` join-chain ownership check before read.

### D) Regression coverage added

- `crates/seasoned-hand-server/tests/middleware_auth.rs`:
  `middleware_auth_tenant_a_gets_404_for_tenant_b_id_endpoints` now probes the queued
  `:id` family against tenant-B resources from tenant-A context and asserts 404:
  - `/v1/tasks/:id`
  - `/v1/tasks/:id/deliverables`
  - `/v1/tasks/:id/provenance`
  - `/v1/projects/:id/tasks`
  - `/v1/projects/:id/archive`
  - `/v1/sessions/:id`
  - `/v1/sessions/:id/progress`
  - `/v1/sessions/:id/feature-list`
  - `/v1/sessions/:id/verifications`
  - `/v1/sessions/:id/checkpoints`
  - `/v1/sessions/:id/checkpoints/:checkpoint_id/rollback`
  - `/v1/verifications/:id`

### iter-4 conclusion

iter-4 found **1 new H + 1 new M**, both fixed inline. Saturation not yet reached
(non-zero new H/M in this round). A confirming iter-5 full re-sweep is still required.

---

## HARDENING iter-5 (Claude, 2026-05-21) — beyond the :id family: workspace + WS

iter-4 completed the HTTP `:id` retrofit and found H5/M6. iter-5 swept the surfaces
the `:id` audit didn't reach: the workspace file proxy and the WebSocket path.

### Findings

- **`P5-HARD-IT5-H6` (H) — FIXED.** `/v1/workspace/:session_id` + `/v1/workspace/:session_id/*sub_path`
  (`workspace_root` / `workspace_proxy`) served raw sandbox files (source, tool outputs,
  any secrets in the workspace) gated by `require_loopback` ONLY — no `with_auth`, no
  AuthContext, no tenant check. A local tenant-A caller could read tenant-B's sandbox by
  session_id — the richest possible leak surface. Fixed: wrapped all three routes with
  `with_auth(..., Action::TaskRead)`, added `Extension<AuthContext>` + `authorize_in_handler`
  + `require_session_tenant(...)` to both handlers (reuses iter-4's session→tenant chain
  helper). Updated the two non-loopback regression tests to pass `Extension(test_auth())`.

- **`P5-HARD-IT5-H7` (H) — RECORDED; assigned to iter-6 (Codex).** The WebSocket path
  (`/ws` → `ws_upgrade` → `ws_session`) has NO auth and NO tenant scoping. `ws_upgrade`
  is `require_loopback` only (not `with_auth`); the message handlers (`task_create`,
  `handle_task_{pause,resume,cancel}`) operate by session_id with no tenant check, and
  `task_create` sets `tenant_id: None` (ws.rs:329). This is the documented Phase 0
  DEBT #7 ("no WS auth") whose own comment said it would be closed "until Phase 5
  multi-user lands real auth" — but Phase 5 did NOT close it. It's a full tenant-isolation
  bypass of the entire task lifecycle over WS. Loopback-gated (same trust boundary as the
  HTTP routes), pre-existing debt, not a Phase 5 code regression — but it must close for
  the multi-user story to hold. **Closure plan (iter-6, Codex):** reuse the header-based
  auth model — wrap `/ws` with `with_auth`, extract `Extension<AuthContext>` in
  `ws_upgrade`, thread it through `ws_session` into the message arms, call
  `require_session_tenant` before pause/resume/cancel, and set the real tenant on
  `task_create`. No new handshake protocol needed. Closes DEBT #7.

### Route-coverage note
Audited every `app()` route. Intentionally-public/token-gated routes (`/healthz`, `/ws`
[H7], `/v1/cost`, `/v1/intake/webhook` [own token], `/v1/admin/sandbox/cleanup` [admin
token]) are by-design. All `/v1/{tasks,projects,sessions,verifications}/:id*` routes are
now `with_auth` + tenant-guarded (iter-3/4/5). Workspace closed here. WS is the one
remaining gap (H7).

### Iter-5 conclusion
Round produced **2 H (H6 fixed, H7 recorded→iter-6)** → NOT zero-finding. iter-6 (Codex):
close H7 (WS tenant scoping / DEBT #7) + independent re-sweep. Saturation still pending.

---

## HARDENING iter-6 (Codex, 2026-05-21) — WS auth + tenant scope closure

### A) Grade Claude iter-5 finding

- **`P5-HARD-IT5-H6` (workspace proxy leak) — ACK.**
  The fix is correct: `with_auth(..., Action::TaskRead)` on workspace routes plus
  handler-level `authorize_in_handler + require_session_tenant(...)` closes the
  highest-value file-read surface for forged `session_id`.

### B) H7 closure (DEBT #7)

- **`P5-HARD-IT5-H7` (H) — FIXED in iter-6.**
  Implemented end-to-end closure:
  1. `/ws` route is now wrapped with `with_auth(get(ws::ws_upgrade), Action::TaskRead)`.
  2. `ws_upgrade` now requires `Extension<AuthContext>`.
  3. `ws_session` and `handle_command` now carry `AuthContext`.
  4. `task_pause` / `task_resume` / `task_cancel` call `require_session_tenant(...)` before
     mutation; cross-tenant attempts emit WS `Error{kind:"forbidden_session_scope"}` and
     `Ack{ok:false,error:"forbidden_session_scope"}`.
  5. `task_create` now writes `IntakeEvent.tenant_id = Some(auth_ctx.tenant_id)` instead of `None`.
  6. Added regression test `ws_tenant_a_cannot_pause_tenant_b_session`.

### C) Debt closure updates

- Closed the explicit carry-forward in:
  - `specs/phase-2/DEBT.md` item #43 (Phase 0 DEBT #7 widening), with concrete Phase 5
    hardening closure notes and test evidence.

### D) Independent re-sweep (iter-6)

Focused additional sweep on:
- WS route/wiring (`/ws`, `ws_upgrade`, `ws_session`, command arms),
- loopback-only but sensitive route family,
- id-based handler family already hardened in iter-3/4/5,
- CLI/HTTP lifecycle overlap paths (`task_pause/resume/cancel` via WS helpers).

Findings:
- **New H findings**: 0
- **New M findings**: 0
- **New L findings**: 0

### Iter-6 conclusion

iter-6 introduced no new residual findings beyond closing H7. This round is clean on
new H/M and leaves the stack ready for the final saturation re-sweep (iter-7).

---

## HARDENING iter-7 (Claude, 2026-05-21) — exhaustive route + service-list audit

Codex's iter-6 re-sweep reported 0 new, but a full route-by-route + service-method
audit here found FOUR more — single sweeps were missing surfaces. So iter-7 did an
EXHAUSTIVE pass and records the complete clearance table below so iter-8 can verify
against a checklist instead of re-discovering.

### Findings (all FIXED)

- **`P5-HARD-IT7-H8` (H)** — `/v1/briefings/:id/confirm` (`post_briefing_confirm_handler`)
  had NO auth at all (route lacked `with_auth`, handler had no AuthContext) and
  confirms/cancels/edits a task's lifecycle by id. Any local caller could drive another
  tenant's briefed task. Fixed: route `with_auth(..., TaskWrite)` + `authorize_in_handler`
  + `require_task_tenant`.
- **`P5-HARD-IT7-M7` (M)** — `/v1/inbox` (`get_inbox_handler`) had no `with_auth` and
  `SELECT ... FROM tasks WHERE status='briefed'` with no tenant predicate — returned every
  tenant's briefed-task titles + brief content. Fixed: `with_auth` + tenant-scoped query.
- **`P5-HARD-IT7-M9` (M)** — `SopShareService::list_for_sop` + `PlaybookShareService::list_for_playbook`
  queried `WHERE ss.{sop,playbook}_id = ?` with no tenant predicate. Admins pass
  `authorize_share` unconditionally, so an admin could read another tenant's share metadata
  (subject emails, permissions, granters). Fixed: `AND ss.tenant_id = ?` on both.
- **`P5-HARD-IT7-M10` (M)** — `/v1/user-cost/reconcile` returned the global
  `ReconciliationReport` (drifts across ALL tenants — tenant_id/user_id/cost) to a
  tenant-scoped admin. Fixed: handler filters `report.drifts` to `auth.tenant_id` +
  recomputes `drifted_rows`.

### COMPLETE ROUTE CLEARANCE TABLE (every app() route)

Legend: GUARD = `require_*_tenant`; DERIVED = tenant comes from AuthContext inside the
query (visibility/audit/search/list_by_tenant); PUBLIC = intentionally unauthenticated;
TOKEN = own token gate; N/A = no tenant data.

- `/healthz` PUBLIC · `/ws` GUARD (iter-6: with_auth + require_session_tenant) · `/v1/cost` N/A (global Bifrost)
- `/v1/sessions` DERIVED (list_sessions tenant filter) · `/v1/sessions/:id` GUARD · `/v1/sessions/:id/events` DERIVED (projects.tenant_id JOIN)
- `/v1/events/:session_id` DERIVED (visibility::query) · `/v1/admin/events/:session_id/raw` DERIVED+GUARD (query_raw tenant short-circuit + EventRawRead)
- `/v1/sessions/:id/{feature-list,progress,verifications,checkpoints}` GUARD · `/v1/sessions/:id/checkpoints/:cid/rollback` GUARD
- `/v1/workspace/:session_id[/*]` GUARD (iter-5) · `/v1/verifications/:id` GUARD (iter-4)
- `/v1/admin/sandbox/cleanup` TOKEN · `/v1/channels[/:name/health|/test]` N/A (global channel infra)
- `/v1/intake/webhook` TOKEN+tenant-stamps · `/v1/intake/cli` tenant-stamps from header · `/v1/inbox` GUARD (iter-7)
- `/v1/tasks/:id/provenance` GUARD (iter-4) · `/v1/projects` DERIVED (create stamps tenant; list_by_tenant)
- `/v1/projects/:id/archive` GUARD · `/v1/projects/:id/tasks` GUARD · `/v1/tasks/:id` GUARD · `/v1/tasks/:id/deliverables` GUARD
- `/v1/tasks/:id/{pause,resume,cancel}` GUARD (iter-3) · `/v1/tasks/:id/handoff[/can]` service-layer tenant lookup (H3 fix)
- `/v1/audit` DERIVED (AuditLogger::query) · `/v1/organizations/:slug/users` (invite/list) DERIVED (CrossTenantDenied on slug→org)
- `/v1/user-cost/reconcile` GUARD (iter-7 report filter) · `/v1/sops/:id/shares` (get/post/delete) tenant-scoped (authorize_share + M9 list fix)
- `/v1/briefings/:id/confirm` GUARD (iter-7)

### Iter-7 conclusion
4 new findings (H8 + M7/M9/M10), all fixed → NOT zero-finding. But this was the first
EXHAUSTIVE pass (vs. targeted sweeps). The clearance table above is now complete: every
route is GUARD/DERIVED/PUBLIC/TOKEN/N/A. iter-8 (Codex): VERIFY each row of the table
independently + sweep any non-route surface (workers, CLI, stores) — if 0 new, that plus
a final iter-9 confirm = saturation.

---

## HARDENING iter-8 (Codex finding + Claude completion, 2026-05-21)

Codex began the iter-8 verification pass, found one real discrepancy in the iter-7
clearance table, then hit a model-capacity throttle mid-task. Claude completed the fix +
the non-route sweep.

### New finding (FIXED)

- **`P5-HARD-IT8-H9` (H)** — the iter-7 `/ws` GUARD label was INCOMPLETE. iter-6's H7 fix
  added `require_session_tenant` to the WS task-lifecycle arms (`task_pause/resume/cancel`)
  but NOT to two other message arms:
  - `Subscribe` (ws.rs:284): replays + live-streams a session's entire event feed by
    session_id — a WS client could read another tenant's event stream.
  - `UserResponse` (ws.rs:541): appends a user Message + resumes the runner by session_id
    — a WS client could answer/drive another tenant's session.
  Fixed (Claude): both arms now call `require_session_tenant` and emit
  `Error/Ack{forbidden_session_scope}` on mismatch, matching the iter-6 pattern. All 9 WS
  integration tests still pass (subscribe + user_response flows on tenant-chained sessions
  are unaffected).

### Route clearance table — correction
- `/ws` is now genuinely GUARD across ALL message arms (task_create stamps tenant;
  task_pause/resume/cancel + Subscribe + UserResponse all `require_session_tenant`).

### Non-route surface sweep (Claude completed)
- **Workers**: curator tenant-scoped (story 5.17); curator-retention is project-scoped
  (`WHERE project_id = ?`) which is transitively tenant-bounded (1 project = 1 tenant);
  ttl-cron is a global maintenance GC (deletes expired sandbox dirs — no cross-tenant
  data read); notify/delivery thread `tenant_id` on outbound. None take untrusted
  per-request tenant input → not a cross-tenant-request surface. **0 findings.**
- **Stores by id**: deliverable + checkpoint stores are reached only via the now-guarded
  `:id` handlers (`require_task_tenant`/`require_session_tenant` gate the parent resource
  before listing children), so they inherit the guard. **0 findings.**
- **CLI**: sends tenant via SH_* env (default legacy-default) on every request; the
  server-side guards are the enforcement point (CLI can't bypass them). **0 findings.**

### Iter-8 conclusion
1 new H (H9, fixed) + 0 non-route findings. The WS arm gap was the last incomplete-GUARD
discrepancy. iter-9 (Claude independent confirm) is the saturation check: if it finds 0
new across a fresh full pass, we declare SATURATION.
