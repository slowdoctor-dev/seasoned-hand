# Phase 5 — Multi-User + Organization

Date: 2026-05-20  
Owner: BMAD Analyst pass

## 1. Goals (what success looks like)

- Enable one Seasoned Hand instance to serve a real 5-person team with explicit organization and
  user boundaries, without cross-user task/state collisions.
- Make authorization enforceable and auditable: every mutating action is role-gated and attributed
  to the acting user.
- Promote shared organizational memory (SOPs/playbooks) safely inside an org while preserving
  tenant isolation across org boundaries.
- Keep operational control deterministic under hand-off and delegation workflows (one user assigns,
  another executes) with complete audit visibility.
- Preserve Phase 0-4 performance and reliability guarantees while introducing tenant defaults,
  per-user cost accounting, and redaction-aware event visibility.

## 2. Non-functional requirements

- `NFR-5.1` **Tenant isolation correctness**  
  For any API/CLI/worker path, cross-tenant read/write leakage rate must be 0 in acceptance suite;
  deny-by-default on missing tenant context.  
  Why this matters: Phase 5's value collapses if org boundaries are porous.

- `NFR-5.2` **Authorization decision latency**  
  Role checks (`admin/user/viewer`) must add <= 10ms p95 and <= 25ms p99 per request on baseline
  environment.  
  Why this matters: access control cannot degrade day-to-day delegation UX.

- `NFR-5.3` **Audit completeness + immutability**  
  100% of delegation/handoff/share/approval/reject operations must emit immutable audit records with
  actor, target, decision, and timestamp (INTEGER microseconds).  
  Why this matters: missing audit events make incidents and billing disputes unresolvable.

- `NFR-5.4` **Per-user cost accounting fidelity**  
  Monthly per-user cost totals must reconcile to session/tool-call source rows within +/-0.5% and
  must be queryable per org + project + user.  
  Why this matters: chargeback and governance fail when totals do not reconcile.

- `NFR-5.5` **Shared-memory consistency budget**  
  SOP/playbook share, unshare, and permission updates must become visible to authorized users within
  5 seconds p95 (eventual consistency upper bound), with monotonic visibility per user session.  
  Why this matters: stale permissions create both security bugs and workflow confusion.

- `NFR-5.6` **Redaction-safe multi-tenant event visibility**  
  Tenant-scoped event feeds must never expose raw secret-bearing args/outputs from another tenant;
  redaction policy must be deterministic and testable at query boundary.  
  Why this matters: Phase 4's single-operator assumption is invalid under multi-tenant access.

- `NFR-5.7` **Config strictness harmonization**  
  All multi-user and auth-related env/config flags must use strict parse semantics (invalid values
  fail fast at boot, no silent coercion).  
  Why this matters: permissive parsing can silently disable security boundaries.

- `NFR-5.8` **Zero-downtime migration posture**  
  V013 tenant-tightening migration (nullable -> NOT NULL defaults + org/user/RBAC surfaces) must be
  forward-appliable on a live Phase 4 database with deterministic backfill and no destructive data
  loss.  
  Why this matters: the tenant flip is the phase headline and cannot require reset/reseed.

## 3. Functional requirements

- `F-5.1` **Organization + user core domain**  
  Introduce first-class organization and user entities with explicit org membership links.  
  Contract pin: every task/session/project row must resolve to exactly one org through tenant/user
  mapping after migration.  
  Why this matters: "multi-user" is undefined without explicit org/user ownership graph.

- `F-5.2` **Tenant defaulting policy**  
  Define deterministic tenant resolution order for all write paths (request context -> actor
  membership -> explicit override if authorized) and fail closed when unresolved.  
  Why this matters: ambiguous tenant resolution causes silent data drift across org boundaries.

- `F-5.3` **Tenant NOT NULL migration (headline schema move)**  
  Ship migration that flips Phase 2+ `tenant_id` surfaces from nullable to NOT NULL with backfill
  defaults and integrity checks, including all Phase 4 curator tables.  
  Why this matters: nullable tenant fields keep accidental global rows possible in production.

- `F-5.4` **RBAC role model (`admin`, `user`, `viewer`)**  
  Define permission matrix for read/write/share/delegate/approve actions by role; include project
  scope overlays where needed.  
  Why this matters: role ambiguity turns policy disputes into code-path guesswork.

- `F-5.5` **Authorization middleware + policy engine**  
  Apply RBAC checks consistently across HTTP, CLI, and internal worker-triggered mutations with
  common policy evaluator surface.  
  Why this matters: duplicated auth logic drifts and creates bypasses.

- `F-5.6` **SOP org-sharing semantics**  
  SOPs become shareable within org by policy, with explicit owner/editor/viewer capabilities and
  immutable version provenance.  
  Why this matters: roadmap requires SOP collaboration, not per-user SOP islands.

- `F-5.7` **Playbook org-sharing semantics**  
  Playbooks become shareable within org with explicit visibility state and approval path for
  high-impact promotion.  
  Closes DEBT #93 (optional governance variants) at least to a policy-surface baseline.  
  Why this matters: shared playbooks are core team-memory value in Phase 5.

- `F-5.8` **Task hand-off workflow**  
  Implement assignment transfer (`from_user -> to_user`) with state-machine-safe transitions,
  notifications, and audit trail.  
  Why this matters: delegation between teammates is a roadmap headline behavior.

- `F-5.9` **Delegation + reassignment audit log**  
  Persist auditable records for who delegated/reassigned what, from which role context, and why
  (optional reason field with normalization).  
  Why this matters: hand-off without attribution breaks accountability.

- `F-5.10` **Per-user cost ledger**  
  Track and expose per-user token/cost aggregates with org/project filters and reconciliation to
  source session rows.  
  Why this matters: shared instances require cost accountability per operator.

- `F-5.11` **Tenant-scoped event access model**  
  Define which event types/fields are visible to `viewer`, `user`, `admin` and ensure cross-tenant
  queries are impossible through session_search or raw event endpoints.  
  Why this matters: events are the richest leak surface in multi-user mode.

- `F-5.12` **Tenant-aware event redaction boundary**  
  Resolve Phase 4 security observation: either (A) redact sensitive Action/Observation payloads at
  write/read boundary for tenant-visible feeds, or (B) explicitly adopt operator-visible raw policy
  with strict admin-only gating and documented risk acceptance. Architect must pin one.  
  Why this matters: leaving this implicit creates silent cross-tenant secret exposure risk.

- `F-5.13` **Session/project/task query scoping**  
  All list/search APIs must enforce org + role + project scope filters by default; no unscoped
  global list endpoints in multi-user mode.  
  Why this matters: accidental broad queries become data leaks at team scale.

- `F-5.14` **Curator tenant boundaries**  
  Curator candidate building/consolidation/archive/retention must execute within tenant boundaries,
  including review queue and retrospective outputs.  
  Why this matters: Curator decisions crossing tenants are security and correctness failures.

- `F-5.15` **Curator threshold policy continuation**  
  Resolve DEBT #92 by deciding adaptive-threshold path (ship or explicitly defer) with tenant-safe
  configuration semantics.  
  Why this matters: static thresholds can underperform across heterogeneous orgs.

- `F-5.16` **Retrospective policy continuation**  
  Resolve DEBT #94 by deciding tiered model-by-size path for retrospectives in org context (ship
  or explicit defer with metrics gate).  
  Why this matters: team-scale retrospective volume can stress Phase 4 single-profile defaults.

- `F-5.17` **Rationale schema evolution tooling baseline**  
  Resolve DEBT #96 by introducing rationale-versioning compatibility policy and validation tooling
  for curator decision payload evolution.  
  Why this matters: evolving rationale shapes without tooling breaks audit and replay consumers.

- `F-5.18` **Global strict-config harmonization**  
  Close DEBT #91 globally: strict parse and fail-fast behavior across remaining non-curator config
  families, not only `SH_CURATOR_*`.  
  Why this matters: partial strictness leaves hidden misconfiguration paths.

- `F-5.19` **FTS tuning continuation gate**  
  Close or re-baseline DEBT #76 using Phase 4 dogfood data; ship tuned weighting policy and
  measurement procedure for matcher/session search relevance in multi-user corpus.  
  Why this matters: poor ranking quality scales into noisy shared-memory retrieval.

- `F-5.20` **Dependency-justification discipline closure**  
  Close DEBT #97 by requiring per-crate justification entries for any Phase 5 net-new dependencies
  and update ARCH §1 addenda accordingly.  
  Why this matters: multi-tenant security surface grows with each dependency.

- `F-5.21` **Invitation/provisioning workflow**  
  Add org user-invite/provision/deactivate lifecycle with role assignment and deactivation-safe
  ownership transfer rules.  
  Why this matters: teams need controlled onboarding/offboarding to avoid orphaned assets.

- `F-5.22` **Shared artifact conflict resolution surface**  
  Provide deterministic behavior when concurrent edits/shares target same SOP/playbook (lock,
  optimistic concurrency, or queue policy selected by Architect).  
  Why this matters: "without stepping on each other" requires explicit conflict semantics.

- `F-5.23` **Org-scoped CLI surfaces**  
  Extend CLI with org/user/role-aware command group(s) for membership, sharing, hand-off, and
  audit inspection in parity with API behavior.  
  Why this matters: operator workflows in this project rely on CLI-first operability.

- `F-5.24` **Phase 5 acceptance harness**  
  Ship reproducible acceptance suite that models a 5-person team using one instance with concurrent
  task delegation, hand-off, shared SOP/playbook usage, and per-user cost/audit verification.  
  Why this matters: roadmap acceptance criterion must be executable, not aspirational prose.

## 4. Story breakdown

Phase 5 PM pass will expand this into concrete stories (target approximately 22-30 stories,
1-3 hours each, mirroring Phase 4 granularity).

| story_id | title | est | deps | status |
|---|---|---:|---|---|
| 5.1 | Phase 5 scaffolds + story map + baseline hooks | 0.5h | — | ready |
| 5.X | PM persona fills full story set in Phase 5 planning pass | TBD | TBD | ready |

## 5. Acceptance criteria (Phase-level)

Phase 5 is accepted when all of the following hold:

1. A 5-person team simulation (1 admin, 3 users, 1 viewer) runs on one instance for a full
   acceptance scenario suite without cross-tenant data leakage or authorization bypass.
2. Shared SOP/playbook workflows support create/share/edit/use/handoff with audit trail present for
   all mutating operations.
3. Per-user cost ledger reconciles to source session/tool-call totals within NFR-5.4 tolerance.
4. Tenant NOT NULL migration applies from Phase 4 baseline DB with deterministic backfill and no
   destructive data loss.
5. Event visibility/redaction policy from F-5.12 is explicitly implemented and tested against
   cross-tenant leakage cases.
6. Phase 4 carry-forward debt set in scope (#76, #91, #92, #93, #94, #96, #97) is marked closed,
   partial with evidence, or explicitly deferred with Phase 6 owner and rationale.

## 6. Out of scope (explicitly deferred)

- Cross-organization federation and marketplace-style artifact exchange (Phase 6+).
- Enterprise SSO/SAML/SCIM and external IdP integration (Phase 6+).
- Fine-grained ABAC policy language beyond `admin/user/viewer` baseline (Phase 6+).
- Organization-to-organization billing and chargeback integration (Phase 6+).
- Public plugin ecosystem trust/rating governance (Phase 6+).
- Advanced legal/compliance retention regimes beyond baseline audit immutability (Phase 6+).

## 7. Risks and mitigations

- **Risk: tenant backfill mistakes create mixed-tenant rows**  
  Mitigation: staged migration checksums, preflight validation, rollback plan, post-migration audit.

- **Risk: RBAC gaps across non-HTTP paths (CLI/workers)**  
  Mitigation: centralized policy evaluator and shared authorization tests per surface.

- **Risk: event payload leakage across tenants**  
  Mitigation: explicit F-5.12 policy, redaction/read filtering tests, admin-only raw exceptions if
  chosen.

- **Risk: hand-off race conditions cause task ownership confusion**  
  Mitigation: explicit state transition guards + optimistic concurrency token checks.

- **Risk: shared artifact edit collisions**  
  Mitigation: deterministic conflict policy and revision history for recovery.

- **Risk: per-user cost attribution drift**  
  Mitigation: reconciliation job + invariant tests against session/tool-call ledgers.

- **Risk: strict config rollout causes boot failures in existing deployments**  
  Mitigation: migration guide + compatibility lint command before enabling hard mode.

- **Risk: Curator tenantization regresses Phase 4 quality/cost**  
  Mitigation: tenant-scoped benchmarks and staged rollout flags with guardrails.

- **Risk: Phase 5 scope creep beyond roadmap**  
  Mitigation: keep work within roadmap deliverables; defer enterprise extras to Phase 6.

## 8. Dependencies (internal + external)

Internal dependencies:

- ARCH v1.3 data model (`events`, `playbooks`, V011 curator tables) as migration baseline.
- Phase 4 production Curator runtime surfaces and telemetry schemas.
- Phase 2+ task/project/session lifecycle and existing CLI/API flows.
- Security hardening outputs in `specs/SECURITY_REVIEW.md` (tenant redaction observation).

Likely architecture dependencies (to be pinned by Architect pass):

- V013 migration (tenant tightening + org/user/RBAC/audit schemas).
- Successor ADR (ADR-014) if ARCH v1.3 -> v1.4 reconciliation is required for schema/event changes.
- Possible auth/policy helper crates (only with DEBT #97 discipline + ARCH §1 addendum updates).

## 9. Open questions linkage

Architect-resolution candidates are tracked in `/specs/phase-5/OPEN_QUESTIONS.md`.
Any requirement above marked as policy-shape-sensitive (notably F-5.3/F-5.4/F-5.12/F-5.14/F-5.22)
must be concretized there before PM story breakdown.
