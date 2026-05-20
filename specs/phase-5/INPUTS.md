# Phase 5 — Inputs

Date: 2026-05-20  
Owner: BMAD Analyst pass

## 1. What Phase 5 ships (ROADMAP contract)

From `specs/06-roadmap/ROADMAP.md` §Phase 5:

- Multi-tenant data model (organization -> user)
- Role-based access (`admin`, `user`, `viewer`)
- SOPs shared across users within an org
- Playbooks shareable
- Task hand-off (one user -> another)
- Audit log (who delegated what)
- Per-user cost tracking

Acceptance target in roadmap: a 5-person team uses one instance without stepping on each other.

## 2. Core philosophy + decision records

Primary anchors:

- `AGENTS.md` §§8-10: spec/code same-slice reconciliation, no silent drift, strict verification.
- `specs/00-philosophy/PRINCIPLES.md`: append-only evidence trail, conservative autonomy,
  failure-visible operations.
- `specs/01-architecture/decisions/ADR-007`: conservative learning guardrails still apply under
  multi-user mode.
- `specs/01-architecture/decisions/ADR-010`: plan-as-PCB structure remains task lifecycle backbone.
- `specs/01-architecture/decisions/ADR-012` + `ADR-013`: atomic schema/spec reconciliation pattern
  for V010/V011; Phase 5 should mirror this discipline for V013+.

## 3. Immutable architecture surfaces Phase 5 must respect

From `specs/01-architecture/ARCHITECTURE.md` v1.3:

- `events` remains append-only and the canonical operational history.
- Skill/Misc event taxonomy currently includes Phase 3/4 learning + curator signals.
- V011 curator schema is baseline reality; tenant forward-compat columns are present and nullable.
- 12-slot model routing stays fixed-shape unless a successor ADR explicitly changes it.

Phase 5 is expected to tighten tenant semantics (nullable -> NOT NULL) and may require ARCH v1.4.

## 4. Schema reality and migration gap

Current state (post-Phase 4):

- `tenant_id` column exists across Phase 2+ domain tables and all Phase 4 curator tables, but many
  remain nullable.
- `playbooks` already has tenant_id from V009; Phase 4 added denormalization/revision graph
  (`source_project_id`, `active_revision_id`, `playbook_revisions`, `playbook_revision_outcomes`).
- No first-class org/user membership/RBAC tables yet.
- Audit domain for delegation/reassignment is not first-class yet.

Phase 5 headline schema move:

- Flip tenant columns to NOT NULL defaults + introduce org/user/membership/role and audit schema in
  one reconciled migration slice.

## 5. Code surfaces likely touched in Phase 5

- `crates/seasoned-hand-server/src/lib.rs`
  - HTTP auth/context extraction, tenant scoping, role checks, audit write-paths.
- `crates/seasoned-hand-server/src/main.rs`
  - boot-time config parsing for auth/tenant policies and worker wiring.
- `crates/seasoned-hand-core/src/project/*`
  - task/project ownership, reassignment/handoff state transitions.
- `crates/seasoned-hand-core/src/events/*`
  - tenant-scoped event query model and potential redaction boundaries.
- `crates/seasoned-hand-core/src/search/*`
  - session search scoping and multi-user access filtering.
- `crates/seasoned-hand-core/src/curator/*`
  - tenant boundaries for cycle inputs/outputs/review queue/retention.
- `crates/seasoned-hand-core/src/config/*` and env parsing helpers
  - DEBT #91 global strict-parse closure.
- `crates/seasoned-hand-cli/src/commands/*`
  - org/user/role/share/handoff/audit command surfaces.
- `migrations/*.sql`
  - V013 tenant tightening + org/user/RBAC/audit schema changes.

## 6. Security review carry-forward

From `specs/SECURITY_REVIEW.md` iter-3 observation:

- Action events can contain raw tool args/outputs in canonical events table.
- Single-operator model tolerated this; multi-tenant model does not.
- Phase 5 must explicitly decide and implement tenant-scoped event redaction/visibility policy.

This is load-bearing and must appear as first-class requirement (F-5.12).

## 7. Phase 4 debt carry-forward into Phase 5

Unclosed/partial entries from `specs/phase-4/DEBT.md` close-out matrix:

- `#76` FTS5 weighting retune — partial
- `#91` global config strict-parse harmonization — partial
- `#92` adaptive auto-archive thresholds — deferred
- `#93` optional fork-promotion governance — deferred
- `#94` retrospective tiered model-by-size — deferred
- `#96` curator rationale schema evolution tooling — deferred
- `#97` per-crate dependency justification discipline — deferred

Phase 5 requirements must map each to close/partial/defer disposition.

## 8. Cross-phase lessons from REVIEW loops

Phase 3 and 4 review loops repeatedly found structural gaps where interfaces existed but production
wiring was missing. For Phase 5 planning, every load-bearing contract should include:

- production runtime owner,
- authorization boundary,
- migration/backfill proof,
- audit/telemetry proof,
- acceptance harness coverage.

## 9. Existing constraints that shape Phase 5 scope

- Stay within roadmap deliverables; do not pre-build Phase 6 enterprise features (SSO/federation).
- Preserve append-only event model and deterministic migration semantics.
- Keep single-instance local-first operability (no mandatory external auth service dependency unless
  explicitly justified).

## 10. Analyst deliverable expectation for this phase

Phase 5 Analyst pass outputs:

- `requirements.md` with >=20 functional requirements and >=5 NFRs.
- `OPEN_QUESTIONS.md` with architecture-level decisions (A/B/C/D tradeoffs).
- `DEBT.md` seeded with carry-forward mapping and closure expectations.
- Story breakdown placeholder table for PM persona expansion.

This file is the pre-Architect index and should be superseded by resolved architecture choices in
`specs/phase-5/architecture.md`.
