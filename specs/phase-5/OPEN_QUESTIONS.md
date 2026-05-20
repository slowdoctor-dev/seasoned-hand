# Phase 5 — Open Questions

Date: 2026-05-20  
Owner: BMAD Analyst pass

This file captures architecture-level tradeoffs the Phase 5 Architect must resolve before PM story
breakdown and implementation.

## 1. Tenant flip strategy (`tenant_id` nullable -> NOT NULL)

Context: Phase 2..4 introduced nullable `tenant_id` for forward-compat. Phase 5 must tighten to
NOT NULL with safe backfill.

### Options

#### A. Single-step migration with hard fail on unresolved rows

Pros:
- Strictest data integrity posture.
- No prolonged mixed semantics window.

Cons:
- Highest migration risk if legacy rows are inconsistent.
- Operational rollback complexity.

#### B. Two-step migration (backfill+validate, then enforce NOT NULL)

Pros:
- Safer rollout with explicit validation checkpoints.
- Better observability for remediation before hard enforcement.

Cons:
- Longer migration sequence.
- Temporary dual-mode logic.

#### C. Keep nullable and enforce in app layer only

Pros:
- Minimal schema disruption.
- Fastest delivery.

Cons:
- Violates Phase 5 headline schema requirement.
- Leaves long-term drift risk.

Suggested next step: Architect resolves and pins V013 order-of-operations.

## 2. Organization/user schema shape

Context: roadmap requires org -> user multi-tenant model; exact table graph is open.

### Options

#### A. `organizations`, `users`, `organization_memberships` (many-to-many)

Pros:
- Flexible for future cross-org memberships.
- Clear ownership boundary for role assignment.

Cons:
- More joins in hot paths.
- Additional policy complexity.

#### B. `users` belongs-to one `organization` (one-to-many)

Pros:
- Simpler authorization queries.
- Lower schema complexity.

Cons:
- Harder future federation or contractor workflows.
- Migration rigidity.

#### C. Hybrid with primary org + optional secondary memberships

Pros:
- Preserves simple default path while allowing edge cases.
- Better long-term extensibility.

Cons:
- Most complex constraints and invariants.
- Higher implementation/testing cost.

Suggested next step: Architect resolves minimal viable shape within Phase 5 scope.

## 3. RBAC policy granularity

Context: roadmap pins roles `admin/user/viewer`, but not operation-level matrix details.

### Options

#### A. Coarse matrix at org level

Pros:
- Fast to implement and explain.
- Low policy-evaluation overhead.

Cons:
- Insufficient for project-level delegation boundaries.
- Can overgrant for shared orgs.

#### B. Org role + project override role

Pros:
- Better control for mixed-sensitivity projects.
- Supports common team structures.

Cons:
- More policy conflict cases.
- Increased UX complexity.

#### C. Coarse matrix now, project overrides deferred to Phase 6

Pros:
- Keeps Phase 5 focused.
- Lower initial risk.

Cons:
- May underdeliver "without stepping on each other" in complex orgs.
- Deferral pressure to Phase 6.

Suggested next step: Architect resolves with acceptance-fit rationale.

## 4. Authorization enforcement architecture

Context: enforcement must cover API, CLI, and worker-triggered mutations.

### Options

#### A. Middleware-only checks per endpoint

Pros:
- Explicit and localized.
- Works with existing route architecture.

Cons:
- Easy to miss non-HTTP paths.
- Policy duplication risk.

#### B. Central policy engine called by all surfaces

Pros:
- Single source of truth.
- Better regression-test leverage.

Cons:
- Refactor overhead.
- Requires disciplined adoption.

#### C. Hybrid (middleware precheck + core policy assertions)

Pros:
- Defense in depth.
- Better protection against bypasses.

Cons:
- Additional complexity.
- Potential latency overhead if poorly implemented.

Suggested next step: Architect resolves and pins shared API contract.

## 5. SOP sharing permission model

Context: SOPs must be shared across users in an org; edit/write governance is open.

### Options

#### A. Owner + org-read default

Pros:
- Simple collaboration model.
- Low friction for SOP discovery.

Cons:
- Risk of uncontrolled SOP proliferation.
- Limited edit governance.

#### B. Owner/editor/viewer ACL per SOP

Pros:
- Fine-grained control.
- Better auditability.

Cons:
- Higher UX and storage complexity.
- More policy edge cases.

#### C. Admin-controlled publication workflow

Pros:
- Strong governance and quality control.
- Reduced accidental policy drift.

Cons:
- Slower iteration.
- Operational bottleneck risk.

Suggested next step: Architect resolves baseline policy + escalation path.

## 6. Playbook sharing governance

Context: Playbooks become shared team memory, but promotion policy can vary.

### Options

#### A. Immediate org-visible on extraction

Pros:
- Fastest learning propagation.
- Minimal friction.

Cons:
- Higher risk of low-quality spread.
- Harder to manage noisy artifacts.

#### B. Confidence-based auto-share + review queue for low confidence

Pros:
- Balances speed and safety.
- Reuses Phase 4 review queue pattern.

Cons:
- Threshold tuning complexity.
- Potential backlog accumulation.

#### C. Manual publish-only

Pros:
- Strong quality gate.
- Predictable governance.

Cons:
- Weak autonomous-learning effect.
- Increased manual burden.

Suggested next step: Architect resolves and maps to DEBT #93 disposition.

## 7. Task hand-off state transition semantics

Context: reassignment may happen while task is Drafted/Running/Paused/etc.

### Options

#### A. Allow hand-off only in non-running states

Pros:
- Lower runtime complexity.
- Fewer race conditions.

Cons:
- Less useful for live operational handoffs.
- May block urgent transfers.

#### B. Allow hand-off in running/paused with checkpoint guard

Pros:
- Supports realistic teamwork.
- Preserves runtime continuity.

Cons:
- Requires stronger coordination semantics.
- More failure modes.

#### C. Running hand-off requires pause->transfer->resume sequence

Pros:
- Deterministic transitions.
- Clear ownership handover moment.

Cons:
- Additional operator steps.
- Slightly slower hand-off.

Suggested next step: Architect resolves lifecycle contract for PM stories.

## 8. Audit log storage model

Context: roadmap requires "who delegated what"; storage shape is open.

### Options

#### A. Dedicated `audit_log` table per operation

Pros:
- Queryable and policy-focused.
- Easier compliance reporting.

Cons:
- Additional persistence surface.
- Potential duplication with events.

#### B. Encode all audit data as `Misc` events only

Pros:
- Reuses append-only stream.
- Fewer tables/migrations.

Cons:
- Harder structured reporting.
- Event payload drift risk.

#### C. Dual-write (`audit_log` + summarized event)

Pros:
- Best of both query/report and timeline UX.
- Supports multiple consumers.

Cons:
- Write-path complexity.
- Idempotency concerns.

Suggested next step: Architect resolves with immutability guarantees.

## 9. Per-user cost accounting source of truth

Context: current totals aggregate at session/task level; per-user rollup design is open.

### Options

#### A. Compute-on-read from sessions + tool_calls + cost

Pros:
- No new write paths.
- Always derived from canonical source.

Cons:
- Expensive for large windows.
- Harder to snapshot monthly reports.

#### B. Incremental ledger table updated at task/session boundaries

Pros:
- Fast reporting queries.
- Better audit snapshots.

Cons:
- Requires reconciliation logic.
- Risk of drift if writer bugs occur.

#### C. Hybrid (materialized rollups + periodic reconciliation)

Pros:
- Good performance with correctness backstop.
- Operationally robust.

Cons:
- Highest complexity.
- More scheduled jobs.

Suggested next step: Architect resolves with NFR-5.4 reconciliation strategy.

## 10. Tenant-scoped event redaction policy (security carry-forward)

Context: Phase 4 left raw Action args/outputs in events; multi-tenant visibility changes risk posture.

### Options

#### A. Redact on write

Pros:
- Secrets never persist in tenant-visible store.
- Strongest leakage prevention.

Cons:
- Loses raw forensic detail for operators.
- Hard to evolve redaction mistakes.

#### B. Store raw, redact on read per role/tenant

Pros:
- Preserves full operator forensics.
- Flexible policy evolution.

Cons:
- Higher enforcement complexity.
- Any query-path bypass becomes critical.

#### C. Dual-store (raw restricted + redacted searchable projection)

Pros:
- Strong separation for security and observability.
- Supports robust tenant-facing search.

Cons:
- More schema and trigger complexity.
- Storage overhead.

Suggested next step: Architect must resolve explicitly; this is load-bearing.

## 11. Session search under RBAC

Context: search currently indexes broad event text; multi-user access scope must be enforced.

### Options

#### A. Query-time filtering only

Pros:
- Minimal migration impact.
- Keeps single index model.

Cons:
- Filtering mistakes leak data.
- Potential performance penalties.

#### B. Tenant-partitioned index tables

Pros:
- Stronger isolation by construction.
- Better predictable query plans.

Cons:
- Migration/storage complexity.
- Harder cross-tenant admin analytics.

#### C. Shared index with tenant+visibility columns and strict compound predicates

Pros:
- Flexible with manageable migration scope.
- Supports admin-only global views.

Cons:
- Requires strict predicate discipline everywhere.
- More complex tests.

Suggested next step: Architect resolves with test strategy.

## 12. Curator tenantization scope

Context: Curator currently uses project scope; Phase 5 must ensure tenant boundaries.

### Options

#### A. Hard tenant partition in all curator queries/writes

Pros:
- Cleanest isolation semantics.
- Easy to reason about security.

Cons:
- Might reduce cross-project learning inside same org if too strict.
- Potential duplication.

#### B. Tenant partition + optional org-wide aggregation mode

Pros:
- Better learning utility inside org.
- Still bounded by tenant.

Cons:
- Additional policy knobs and tests.
- Risk of accidental overreach.

#### C. Keep project-only for Phase 5, defer org aggregation

Pros:
- Lowest risk rollout.
- Minimal change from Phase 4.

Cons:
- Underdelivers shared-memory value.
- Defers core roadmap benefit.

Suggested next step: Architect resolves with acceptance fit.

## 13. Global strict-config harmonization rollout

Context: DEBT #91 is partially closed (curator-only); broader configs remain mixed strictness.

### Options

#### A. Big-bang strict parse for all config families

Pros:
- Uniform behavior quickly.
- Simpler long-term maintenance.

Cons:
- Higher rollout breakage risk.
- Large blast radius.

#### B. Security-critical first, others phased

Pros:
- Prioritizes high-impact flags.
- Controlled adoption.

Cons:
- Mixed semantics persist temporarily.
- Requires clear phase boundaries.

#### C. Strict lint command + opt-in enforcement, then default strict

Pros:
- Safer migration experience.
- Better operator readiness.

Cons:
- Longer path to full closure.
- Additional tooling work.

Suggested next step: Architect resolves migration posture.

## 14. Concurrency control for shared artifacts

Context: multiple users may edit/share same SOP/playbook concurrently.

### Options

#### A. Last-write-wins

Pros:
- Simple implementation.
- No blocking UX.

Cons:
- High overwrite risk.
- Poor collaboration trust.

#### B. Optimistic concurrency (revision/version checks)

Pros:
- Preserves integrity with clear conflict detection.
- Aligns with revision-chain model.

Cons:
- Requires conflict resolution UX.
- Slightly more complex APIs.

#### C. Explicit locks with timeout

Pros:
- Clear editing ownership windows.
- Reduces merge conflicts.

Cons:
- Lock contention and stale lock handling.
- Higher operational friction.

Suggested next step: Architect resolves baseline strategy.

## 15. Org provisioning and offboarding behavior

Context: user lifecycle affects ownership of tasks/SOPs/playbooks/audit accountability.

### Options

#### A. Soft-deactivate users, keep ownership unchanged

Pros:
- Preserves historical fidelity.
- Simple implementation.

Cons:
- Leaves active ownership orphan risks.
- Admin cleanup burden.

#### B. Deactivate + mandatory ownership reassignment workflow

Pros:
- Prevents orphaned active artifacts.
- Cleaner operations.

Cons:
- Extra offboarding complexity.
- Requires robust tooling.

#### C. Archive user-owned mutable assets on deactivation

Pros:
- Conservative safety posture.
- Easy conflict avoidance.

Cons:
- May disrupt active workstreams.
- Recovery overhead.

Suggested next step: Architect resolves lifecycle contract.

## 16. Architecture versioning and ADR boundary for Phase 5

Context: tenant flip + org/RBAC schema likely changes ARCH §2.5 and possibly event taxonomy.

### Options

#### A. Keep ARCH v1.3 and document in Phase 5 specs only

Pros:
- Less immediate documentation churn.
- Faster initial implementation.

Cons:
- Violates prior reconciliation discipline.
- Increases drift risk.

#### B. Bump ARCH v1.3 -> v1.4 with successor ADR in atomic migration slice

Pros:
- Preserves architecture-as-source-of-truth discipline.
- Clear historical lineage.

Cons:
- Additional documentation workload.
- Requires careful same-PR choreography.

#### C. Split ADR and ARCH bump across multiple PRs

Pros:
- Smaller PRs.
- Easier per-change review.

Cons:
- Conflicts with established atomic-slice reconciliation pattern.
- Drift windows likely.

Suggested next step: Architect resolves; default should mirror ADR-012/013 discipline.
