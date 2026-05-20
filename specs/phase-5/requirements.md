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
  For any API/CLI/worker path, cross-tenant read/write leakage rate must be 0 in the
  `phase5_cross_tenant_isolation_harness` acceptance suite (named by analogy with
  Phase 4's `phase4_warm_full_loop_benchmark`); deny-by-default on missing tenant
  context. Harness must enumerate the API surface, CLI surface, and every spawned
  worker (verifier, curator, retention, ttl, notify, intake) and assert each
  rejects a forged cross-tenant request.  
  Why this matters: Phase 5's value collapses if org boundaries are porous, and
  "0 leakage" needs a named, runnable target — not prose.

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
  All env/config flags must use strict parse semantics (invalid values fail fast at
  boot, no silent coercion). Scope is global — closes DEBT #91 across all non-curator
  config families, not just the multi-user/auth subset. Curator scope was already
  closed in story 4.14; Phase 5 finishes the harmonization.  
  Why this matters: permissive parsing can silently disable security boundaries
  anywhere, not only on auth flags.

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
  source session rows. **Tracking only** — per-user spending **caps** / hard-stops are explicitly
  out of scope (see §6); roadmap pins "tracking", not enforcement.  
  Why this matters: shared instances require cost accountability per operator; conflating tracking
  with enforcement invites unbounded scope expansion at PM time.

- `F-5.11` **Tenant-scoped event access model**  
  Define which event types/fields are visible to `viewer`, `user`, `admin` and ensure cross-tenant
  queries are impossible through session_search or raw event endpoints.  
  Why this matters: events are the richest leak surface in multi-user mode.

- `F-5.12` **Tenant-aware event redaction boundary**  
  Resolve the Phase 4 security observation recorded in `/specs/SECURITY_REVIEW.md` iter-3
  saturation section ("Phase 4 Action events stored in the canonical events table contain raw
  tool args + outputs without applying `redact_pii`. When Phase 5 flips `tenant_id` to NOT NULL
  and the events table starts carrying multi-tenant rows, this becomes a real cross-tenant leak
  surface."). Architect picks one of: (A) redact sensitive Action/Observation payloads at
  write boundary for tenant-visible feeds, (B) store raw + redact on read per role/tenant,
  (C) dual-store (raw restricted + redacted searchable projection). See OPEN_QUESTIONS §10.  
  Why this matters: leaving this implicit creates silent cross-tenant secret exposure risk.

- `F-5.13` **Session/project/task query scoping**  
  All list/search APIs must enforce org + role + project scope filters by default; no unscoped
  global list endpoints in multi-user mode.  
  Why this matters: accidental broad queries become data leaks at team scale.

- `F-5.14` **Curator tenant boundaries**  
  Curator candidate building/consolidation/archive/retention must execute within tenant
  boundaries, including review queue and retrospective outputs. Failure taxonomy when
  tenant resolution fails mid-cycle (pinned here so PM doesn't have to re-derive it):
  (a) MISSING tenant_id on a candidate row → quarantine that decision unit with
  `failure_category="tenant_unresolved"`, continue the rest of the cycle;
  (b) CROSS-tenant reference detected (decision references a row from another
  tenant) → reject before write, emit `curator_decision_quarantined` with
  `failure_category="cross_tenant_ref"`;
  (c) ENTIRE cycle fails tenant gating at startup → emit
  `curator_cycle_refused` Misc event and skip the tick (analogous to F-4.11
  budget-circuit-open).
  Why this matters: Curator decisions crossing tenants are security and correctness
  failures; un-enumerated failure modes get reinvented per implementer.

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

Phase 5 PM pass expanded this into 33 stories (1-3 hours each, mirroring Phase 4 granularity).
Atomic-slice story is 5.2 (V013 + ADR-014 + ARCH v1.4 — same shape as Phase 4 story 4.2).

| story_id | title | est | deps | status |
|---|---|---:|---|---|
| 5.1 | Phase 5 scaffolds + story map + baseline hooks | 0.5h | — | done |
| 5.2 | Atomic slice: V013 + ADR-014 + ARCH v1.4 + tenant backfill | 3.0h | 5.1 | done |
| 5.3 | AuthContext resolver + Policy engine core | 3.0h | 5.2 | done |
| 5.4 | org/user/membership persistence + project_role_overrides | 2.5h | 5.2 | done |
| 5.5 | HTTP middleware RBAC enforcement | 2.0h | 5.3, 5.4 | done |
| 5.6 | CLI + worker RBAC enforcement (hybrid defense) | 2.5h | 5.3, 5.4 | done |
| 5.7 | sop_shares ACL + CLI surfaces | 2.5h | 5.4, 5.5 | done |
| 5.8 | playbook_shares + visibility_state + curator integration | 3.0h | 5.4, 5.5 | done |
| 5.9 | Task hand-off lifecycle (pause -> transfer -> resume) | 3.0h | 5.5, 5.6 | done |
| 5.10 | audit_log writer + admin read API | 2.5h | 5.4, 5.5 | done |
| 5.11 | Hand-off audit emission + handoff CLI | 2.0h | 5.9, 5.10 | done |
| 5.12 | user_cost_ledger nearline writer | 2.5h | 5.4 | done |
| 5.13 | user_cost reconciliation job + drift alarm | 2.0h | 5.12 | ready |
| 5.14 | tenant_event_view projection + write-time redaction hook | 3.0h | 5.2 | ready |
| 5.15 | session_search_index RBAC predicates + redacted source | 3.0h | 5.14 | ready |
| 5.16 | events::visibility module + admin raw-event route | 2.5h | 5.14, 5.5 | ready |
| 5.17 | Curator tenant boundaries + failure taxonomy | 3.0h | 5.2, 5.4 | ready |
| 5.18 | Optional org-wide curator aggregation flag (default off) | 1.5h | 5.17 | ready |
| 5.19 | User invitation CLI + provisioning | 2.5h | 5.4, 5.10 | ready |
| 5.20 | User deactivation + mandatory reassignment | 2.5h | 5.19, 5.9 | ready |
| 5.21 | Optimistic concurrency for shared artifacts | 2.0h | 5.7, 5.8 | ready |
| 5.22 | Global strict-config harmonization (closes DEBT #91) | 2.0h | 5.2 | ready |
| 5.23 | Per-crate dependency justification (closes DEBT #97) | 1.0h | 5.1 | ready |
| 5.24 | FTS5 weight retune (closes/partial DEBT #76) | 2.5h | 5.15 | ready |
| 5.25 | Curator rationale schema versioning + DEBT #92/#93/#94/#96 decisions | 2.5h | 5.17 | ready |
| 5.26 | phase5_cross_tenant_isolation_harness (NFR-5.1) | 3.0h | 5.5, 5.6, 5.17 | ready |
| 5.27 | phase5_rbac_matrix_harness (NFR-5.2) | 2.0h | 5.5, 5.6 | ready |
| 5.28 | phase5_handoff_lifecycle_harness + phase5_curator_tenant_failure_harness | 3.0h | 5.9, 5.17 | ready |
| 5.29 | phase5_event_redaction_visibility_harness + phase5_search_rbac_harness | 3.0h | 5.14, 5.15 | ready |
| 5.30 | phase5_user_cost_reconciliation_harness (NFR-5.4) | 2.0h | 5.13 | ready |
| 5.31 | phase5_v013_migration_harness (NFR-5.8) | 2.5h | 5.2 | ready |
| 5.32 | phase5_team_simulation_benchmark (5-actor, ≤60 min CI budget) | 3.0h | 5.26-5.31 | ready |
| 5.33 | Phase 5 acceptance gate + close-out | 2.5h | 5.2-5.32 | ready |

## 5. Acceptance criteria (Phase-level)

Phase 5 is accepted when all of the following hold:

1. A 5-person team simulation (1 admin, 3 users, 1 viewer) runs on one instance for a full
   acceptance scenario suite without cross-tenant data leakage or authorization bypass. Total
   wall-clock CI budget ≤ 60 min on baseline runner (15 min above Phase 4 §1's 45 min because
   the 5-actor concurrency adds genuine fanout cost; Architect may relax with rationale if
   genuinely infeasible).
2. Shared SOP/playbook workflows support create/share/edit/use/handoff with audit trail present for
   all mutating operations.
3. Per-user cost ledger reconciles to source session/tool-call totals within NFR-5.4 tolerance.
4. Tenant NOT NULL migration applies from Phase 4 baseline DB with deterministic backfill and no
   destructive data loss.
5. Event visibility/redaction policy from F-5.12 is explicitly implemented and tested against
   cross-tenant leakage cases.
6. Phase 4 carry-forward debt set in scope (#76, #91, #92, #93, #94, #96, #97) is marked closed,
   partial with evidence, or explicitly deferred with Phase 6 owner and rationale. The
   `#S-1` security carry-in (tenant-scoped event redaction; see DEBT.md) ships its F-5.12
   resolution path before Phase 5 close-out.

## 6. Out of scope (explicitly deferred)

- Cross-organization federation and marketplace-style artifact exchange (Phase 6+).
- Enterprise SSO/SAML/SCIM and external IdP integration (Phase 6+).
- Fine-grained ABAC policy language beyond `admin/user/viewer` baseline (Phase 6+).
- Organization-to-organization billing and chargeback integration (Phase 6+).
- Public plugin ecosystem trust/rating governance (Phase 6+).
- Advanced legal/compliance retention regimes beyond baseline audit immutability (Phase 6+).
- **Per-user spending caps / hard-stop budgets** — Phase 5 ships per-user cost **tracking**
  (F-5.10) per the roadmap, not enforcement. Cap-and-throttle policy belongs to Phase 6+ if
  operator demand exists.

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
