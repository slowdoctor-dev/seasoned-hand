# Phase 5 — Multi-User + Organization Architecture

Date: 2026-05-20  
Owner: BMAD Architect pass
Status: v1.0 (initial Architect baseline)

## 1. Goals, Scope, and Philosophical Fit

Phase 5 turns Seasoned Hand from single-operator runtime into a shared organizational runtime while
preserving the project's architectural invariants:

- append-only operational evidence (`events` as canonical stream),
- conservative automation with explicit failure visibility,
- deterministic migration discipline (spec/schema/ADR reconcile in same slice),
- local-first operability.

Phase 5 does **not** introduce enterprise federation or external identity-provider complexity. It
implements the roadmap contract only:

- org/user domain,
- role-based access (`admin`, `user`, `viewer`),
- shared SOP/playbooks inside org,
- hand-off + delegation audit,
- per-user cost accounting,
- tenant-safe event access/redaction policy.

Phase 5 also closes the Phase 4 security carry-forward: Action/Observation payload visibility must
be tenant-safe under NOT NULL tenant semantics.

## 2. Component Diagram and Integration Points

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ API + CLI Surfaces                                                           │
│ - HTTP routes (Axum)                                                         │
│ - CLI commands (seasoned-hand org/user/role/share/audit/hand-off)           │
└───────────────────────────────┬──────────────────────────────────────────────┘
                                │ request context (actor, org, tenant)
                                ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ AuthContext Resolver                                                        │
│ - resolves actor_user_id, org_id, tenant_id, role                           │
│ - fail-closed on unresolved tenant                                           │
└───────────────────────────────┬──────────────────────────────────────────────┘
                                │
                                ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ Policy Engine (RBAC core)                                                   │
│ - authorize(action, resource, actor_context)                                │
│ - reusable from HTTP, CLI, and workers                                      │
└───────────────────────────────┬──────────────────────────────────────────────┘
                                │ allowed
        ┌───────────────────────┼───────────────────────────────────────────┐
        ▼                       ▼                                           ▼
┌───────────────┐       ┌──────────────────┐                      ┌──────────────────┐
│ Domain Writes │       │ Query Filters     │                      │ Worker Guards     │
│ - task handoff│       │ - session/task    │                      │ - verifier        │
│ - sharing ACL │       │ - search index    │                      │ - curator         │
│ - memberships │       │ - audit feeds     │                      │ - retention/ttl   │
└───────┬───────┘       └────────┬──────────┘                      └────────┬─────────┘
        │                        │                                          │
        ▼                        ▼                                          ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ Persistence                                                                  │
│ - V013 org/user/membership/role tables                                      │
│ - NOT NULL tenant_id across Phase 2-4 surfaces                              │
│ - audit_log + summarized audit events                                       │
│ - tenant-scoped event visibility / redacted search projection               │
└──────────────────────────────────────────────────────────────────────────────┘
```

New Phase 5 modules:

- `auth::context`: tenant/actor resolution and strict validation.
- `auth::policy`: central RBAC evaluator.
- `org::membership`: org/user role lifecycle.
- `sharing::sop` + `sharing::playbook`: ACL-aware sharing state transitions.
- `handoff::task`: reassignment orchestration + pause/resume guard flow.
- `audit::ledger`: immutable audit records + event summary writer.
- `billing::user_cost`: per-user rollup and reconciliation.
- `events::visibility`: tenant-safe read projection/redaction filter.

## 3. Data Model and V013 Migration Shape

### 3.1 Chosen migration posture

Chosen from OQ #1: **Option B (two-step migration in one V013 script)**.

- Step A: deterministic backfill and validation tables/indexes.
- Step B: enforce NOT NULL tenant semantics by table-rebuild pattern where needed.

This preserves SQLite compatibility with prior migration patterns (V004/V010/V011 style) while
keeping the atomic-slice rule intact.

### 3.2 New tables (V013)

```sql
-- Organizations (tenant boundary root)
CREATE TABLE organizations (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL UNIQUE,
  slug TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('active','suspended','archived')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- Users (account identity; local-first auth model)
CREATE TABLE users (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  email TEXT NOT NULL,
  display_name TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('active','deactivated')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(tenant_id, email)
);

-- Memberships (many-to-many; one primary membership)
CREATE TABLE organization_memberships (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('admin','user','viewer')),
  is_primary INTEGER NOT NULL DEFAULT 0 CHECK(is_primary IN (0,1)),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(organization_id, user_id)
);

CREATE UNIQUE INDEX idx_membership_primary_per_user
  ON organization_memberships(user_id)
  WHERE is_primary = 1;

-- Project role overrides (optional, bounded Phase 5 granularity)
CREATE TABLE project_role_overrides (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('admin','user','viewer')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(project_id, user_id)
);

-- Audit ledger (immutable operation-grade record)
CREATE TABLE audit_log (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id),
  actor_user_id TEXT NOT NULL REFERENCES users(id),
  action TEXT NOT NULL,
  resource_type TEXT NOT NULL,
  resource_id TEXT NOT NULL,
  target_user_id TEXT REFERENCES users(id),
  decision TEXT,
  reason TEXT,
  metadata TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE INDEX idx_audit_tenant_time ON audit_log(tenant_id, created_at DESC);
CREATE INDEX idx_audit_actor_time ON audit_log(actor_user_id, created_at DESC);

-- Per-user monthly cost rollups (materialized)
CREATE TABLE user_cost_ledger (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id),
  user_id TEXT NOT NULL REFERENCES users(id),
  month_yyyymm TEXT NOT NULL,
  session_count INTEGER NOT NULL DEFAULT 0,
  tool_calls INTEGER NOT NULL DEFAULT 0,
  cost_cents INTEGER NOT NULL DEFAULT 0,
  source_low_watermark_event_id INTEGER,
  source_high_watermark_event_id INTEGER,
  reconciled_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(tenant_id, user_id, month_yyyymm)
);
```

### 3.3 Sharing tables

```sql
-- SOP sharing ACL
CREATE TABLE sop_shares (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  sop_id TEXT NOT NULL REFERENCES sops(id) ON DELETE CASCADE,
  subject_type TEXT NOT NULL CHECK(subject_type IN ('org','user')),
  subject_id TEXT NOT NULL,
  permission TEXT NOT NULL CHECK(permission IN ('viewer','editor','owner')),
  granted_by_user_id TEXT NOT NULL REFERENCES users(id),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(sop_id, subject_type, subject_id)
);

-- Playbook sharing ACL
CREATE TABLE playbook_shares (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  playbook_id TEXT NOT NULL REFERENCES playbooks(id) ON DELETE CASCADE,
  subject_type TEXT NOT NULL CHECK(subject_type IN ('org','user')),
  subject_id TEXT NOT NULL,
  permission TEXT NOT NULL CHECK(permission IN ('viewer','editor','owner')),
  visibility_state TEXT NOT NULL CHECK(visibility_state IN ('review','shared','suspended')),
  granted_by_user_id TEXT NOT NULL REFERENCES users(id),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(playbook_id, subject_type, subject_id)
);
```

### 3.4 Tenant NOT NULL flip targets

V013 tightens `tenant_id` to NOT NULL across all Phase 2-4 mutable domain tables and indexes where
missing. Scope includes at minimum:

- `projects`, `tasks`, `deliverables`, `intake_events`, `deliveries`, `notifications`,
  `skills`, `playbooks`, `sessions`.
- Curator tables from V011/V012:
  `playbook_revisions`, `playbook_revision_outcomes`, `curator_decisions`,
  `curator_review_queue`, `sop_conflicts`, `knowledge_items`, `datasource_items`,
  `weekly_retrospectives`, `retrospective_citations`, `curator_search_index`,
  `curator_decisions_summary`.

SQLite implementation pattern uses create-copy-rename for tables where direct ALTER is insufficient.

### 3.5 Backfill defaults and integrity checks

Backfill policy:

1. Derive tenant from primary join path where available:
   - `tasks` from `projects.tenant_id`,
   - `deliverables` from `tasks.tenant_id`,
   - curator artifacts from linked `playbooks/tasks`.
2. Fallback unresolved rows into deterministic sentinel org (`tenant_id='legacy-default'`) and emit
   audit warning rows for remediation.
3. Validate post-backfill:
   - zero NULL tenant rows,
   - FK-consistent tenant chains (resource tenant matches parent tenant),
   - no cross-tenant refs inside curator/revision chains.

Integrity check SQL examples:

```sql
SELECT COUNT(*) FROM tasks WHERE tenant_id IS NULL;
SELECT COUNT(*)
FROM tasks t JOIN projects p ON p.id = t.project_id
WHERE t.tenant_id <> p.tenant_id;
SELECT COUNT(*)
FROM curator_decisions cd
JOIN playbooks p ON p.id = cd.subject_id
WHERE cd.tenant_id <> p.tenant_id;
```

## 4. Authorization Architecture

### 4.1 Chosen RBAC shape

Chosen from OQ #3: **Option B (org role + project override role)**.

- Base role from `organization_memberships.role`.
- Optional narrower or broader project-specific override in `project_role_overrides`.
- Effective role resolver: `effective_role(user, project) = override(role) else org(role)`.

### 4.2 Chosen enforcement architecture

Chosen from OQ #4: **Option C (hybrid)**.

- HTTP middleware enforces context presence and baseline role gate.
- Core domain service re-checks via shared policy engine before mutation.
- CLI and worker paths call same policy engine directly.

This prevents bypass via non-HTTP execution surfaces.

### 4.3 Policy matrix (baseline)

| Action | viewer | user | admin |
|---|---|---|---|
| Read project/task/session in scope | allow | allow | allow |
| Create/update task in assigned project | deny | allow | allow |
| Hand-off task to another user | deny | allow (same org) | allow |
| Approve/reject shared playbook promotion | deny | deny | allow |
| Share SOP/playbook to org | deny | allow (if owner/editor) | allow |
| Manage memberships/roles | deny | deny | allow |
| View audit log (org) | deny | allow (limited) | allow (full) |
| View raw unredacted events | deny | deny | allow (guarded route) |

### 4.4 Context contract (type sketch)

```rust
pub struct AuthContext {
    pub tenant_id: String,
    pub organization_id: String,
    pub actor_user_id: String,
    pub org_role: Role,
    pub project_override_role: Option<Role>,
}

pub enum Role { Admin, User, Viewer }

pub enum Action {
    TaskRead,
    TaskWrite,
    TaskHandoff,
    SopShare,
    PlaybookShare,
    MembershipManage,
    AuditRead,
    EventRawRead,
}
```

## 5. Task Hand-off Lifecycle

Chosen from OQ #7: **Option C (pause -> transfer -> resume for running tasks)**.

Rules:

- `Drafted/Briefed/Confirmed/Paused`: direct reassignment allowed.
- `Running`: system enforces pause first (or denies with actionable error + suggested pause).
- `Completed/Failed/Cancelled`: reassignment denied.

State transition contract:

1. Request `task_handoff(task_id, from_user, to_user)`.
2. Policy check: actor can reassign both users in tenant scope.
3. If running: pause session, append `task_paused_for_handoff` event.
4. Update task owner and assignment fields atomically.
5. Emit audit_log row + `Misc{kind:"task_handoff_completed"}`.
6. Optional resume by new owner.

## 6. SOP and Playbook Sharing

### 6.1 SOP model

Chosen from OQ #5: **Option B (ACL per SOP)**.

- `sop_shares` governs viewer/editor/owner.
- Default on create: owner only + optional org viewer grant configurable.
- Admin can escalate/revoke.

### 6.2 Playbook model

Chosen from OQ #6: **Option B (confidence-based auto-share + review queue)**.

- Curator can auto-share high-confidence artifacts into org visibility state `shared`.
- Low-confidence artifacts remain `review` until admin approval.
- `playbook_shares.visibility_state` governs publication status.

DEBT #93 closure posture:

- Phase 5 baseline closes policy surface.
- Optional stricter fork-promotion-only workflow remains configurable mode, not default.

## 7. Tenant-Scoped Event Redaction Boundary

Chosen from OQ #10: **Option C (dual-store)**.

- Canonical `events` table remains raw (operator-grade forensic source).
- New tenant-visible projection (`tenant_event_view`) stores redacted payload text and visibility
  flags.
- Search and user-facing feeds query projection, never raw table directly.
- Admin-only raw read route exists and is explicitly gated by `Action::EventRawRead`.

Projection schema sketch:

```sql
CREATE TABLE tenant_event_view (
  event_id INTEGER PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
  tenant_id TEXT NOT NULL,
  visibility_level TEXT NOT NULL CHECK(visibility_level IN ('viewer','user','admin')),
  redacted_data TEXT NOT NULL,
  searchable_text TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE INDEX idx_tenant_event_view_tenant_time
  ON tenant_event_view(tenant_id, created_at DESC);
```

Redaction path:

- Write-time hook attempts deterministic redaction (`redact_pii` family + tool-arg scrub patterns).
- If redaction fails, event is quarantined from tenant projection and emits
  `Misc{kind:"tenant_event_projection_failed"}`.

Security consequence:

- Cross-tenant leakage risk drops to projection-policy correctness, not every consumer query path.

## 8. Curator Tenantization

Chosen from OQ #12: **Option B (tenant partition + optional org-wide aggregation mode OFF by default)**.

Baseline behavior:

- All curator reads/writes include strict `tenant_id = :tenant` filter.
- org-wide aggregation flag exists but defaults false and requires admin explicit enablement.

Failure taxonomy from F-5.14 is adopted as concrete runtime contract:

- `tenant_unresolved`: decision unit quarantined; cycle continues.
- `cross_tenant_ref`: write rejected; quarantine event emitted.
- `curator_cycle_refused`: startup tenant gate fails; skip cycle tick.

These are emitted as `Misc` with deterministic payload keys:

```json
{"kind":"curator_decision_quarantined","failure_category":"cross_tenant_ref","tenant_id":"..."}
{"kind":"curator_cycle_refused","failure_category":"tenant_unresolved","tenant_id":"..."}
```

## 9. Per-User Cost Accounting

Chosen from OQ #9: **Option C (materialized rollups + reconciliation)**.

- Nearline writer updates `user_cost_ledger` on session close / periodic checkpoint.
- Daily reconciliation job recomputes from source (`sessions`, `events`) and flags drift >0.5%.

This satisfies NFR-5.4 performance and correctness simultaneously.

## 10. Session Search Under RBAC

Chosen from OQ #11: **Option C (shared index + strict tenant/visibility predicates)**.

- Keep shared `session_search_index` and `session_search_fts` for migration stability.
- Add `tenant_id` + `visibility_level` columns to `session_search_index` in V013.
- All query builders must include compound predicate:

```sql
WHERE tenant_id = :tenant
  AND visibility_level IN (:allowed_levels)
  AND session_id IN (scoped session set)
```

Regression tests must verify forged tenant/session filters return zero rows.

## 11. Concurrency and Conflict Semantics

Chosen from OQ #14: **Option B (optimistic concurrency)**.

- SOP/playbook updates carry `expected_updated_at` (or revision id) precondition.
- On mismatch: `409 conflict` with current revision metadata.
- No hard locks in Phase 5 baseline.

Benefits:

- deterministic and scalable,
- integrates with revision-chain model,
- avoids lock lifecycle complexity.

## 12. User Provisioning Lifecycle

Chosen from OQ #15: **Option B (deactivate + mandatory ownership reassignment)**.

Lifecycle:

- Invite -> active membership with role.
- Deactivate requires reassignment of active task ownership and owner-level shares.
- Historical audit ownership remains unchanged for immutable records.

This avoids orphaned mutable assets while preserving forensic attribution.

## 13. ARCH v1.3 -> v1.4 Amendments

Phase 5 requires ARCH reconciliation in same atomic slice as V013 and ADR-014:

- Status line `v1.3` -> `v1.4`.
- §2.5 amendment paragraph for V013:
  - org/user/membership/role tables,
  - audit log + per-user cost ledger,
  - tenant tightening (NOT NULL) across Phase 2-4 surfaces,
  - tenant event projection surface.
- §2.1 event taxonomy note for new multi-user/audit kinds where needed.
- §1 addendum note for any net-new dependencies (if adopted during implementation stories).

No silent drift windows: V013 SQL + ADR-014 + ARCH v1.4 land together.

## 14. Open Question Resolution Table

| OQ | Chosen | Rationale | Deferred debt |
|---|---|---|---|
| #1 tenant flip | B | safest deterministic rollout with validation checkpoints | none |
| #2 org/user shape | A | flexible many-to-many without overfitting contractor edge cases | none |
| #3 RBAC granularity | B | org baseline + project override fits team reality | none |
| #4 enforcement architecture | C | defense-in-depth across API/CLI/workers | none |
| #5 SOP sharing | B | ACL precision needed for team collaboration | none |
| #6 playbook sharing | B | preserve autonomy with review safety band | #93 fully closes policy surface; strict manual-only mode optional |
| #7 handoff transitions | C | deterministic running-task transfer semantics | none |
| #8 audit storage | C | queryable ledger + timeline visibility both required | none |
| #9 user cost source | C | fast reporting with reconciliation correctness | none |
| #10 event redaction | C | strongest tenant safety with operator forensics retained | none |
| #11 search under RBAC | C | manageable migration with strict predicates | none |
| #12 curator tenantization | B | tenant-safe baseline with optional org aggregation disabled by default | #92 policy tuning remains data-driven |
| #13 strict-config rollout | B | security-critical first while closing global scope in-phase | #91 closes in Phase 5 |
| #14 conflict semantics | B | optimistic concurrency aligns with revision model | none |
| #15 provisioning | B | avoids orphaned ownership; keeps audit fidelity | none |
| #16 ARCH/ADR boundary | B | preserves atomic reconciliation discipline from ADR-012/013 | none |

## 15. Acceptance and Verification Harness Contract

The following harnesses must exist in Phase 5 story set with explicit assertions:

1. `phase5_cross_tenant_isolation_harness`
- Asserts zero cross-tenant leakage on API, CLI, verifier, curator, retention, ttl, notify, intake.
- Includes forged-tenant context attempts and missing-context fail-closed checks.

2. `phase5_rbac_matrix_harness`
- Asserts `admin/user/viewer` matrix decisions for read/write/share/handoff/audit paths.
- Covers project-role override precedence.

3. `phase5_handoff_lifecycle_harness`
- Asserts running-task handoff requires pause->transfer->resume sequence.
- Confirms immutable audit row and task owner update atomicity.

4. `phase5_event_redaction_visibility_harness`
- Asserts tenant-visible feeds return redacted projection, not raw secrets.
- Asserts admin-only raw path is role-gated and logged.

5. `phase5_user_cost_reconciliation_harness`
- Asserts monthly per-user totals reconcile to source rows within +/-0.5%.
- Injects drift scenario and asserts reconciliation alarm.

6. `phase5_v013_migration_harness`
- Applies V013 from Phase 4 baseline DB fixture.
- Asserts: no NULL tenant rows, tenant chain integrity, org/user/membership bootstrap success,
  and rollback-safe deterministic behavior.

7. `phase5_curator_tenant_failure_harness`
- Asserts `tenant_unresolved`, `cross_tenant_ref`, `curator_cycle_refused` categories emit and
  quarantine behavior is correct.

8. `phase5_search_rbac_harness`
- Asserts session search and FTS endpoints enforce tenant + visibility predicates under forged input.

### Requirement coverage anchor

- F-5.1/F-5.2/F-5.3/F-5.13 + NFR-5.1/NFR-5.8 -> harnesses 1, 6.
- F-5.4/F-5.5/F-5.23 + NFR-5.2 -> harness 2.
- F-5.8/F-5.9/F-5.21/F-5.22 + NFR-5.3/NFR-5.5 -> harnesses 3, 2.
- F-5.11/F-5.12 + NFR-5.6 -> harness 4.
- F-5.10 + NFR-5.4 -> harness 5.
- F-5.14/F-5.15/F-5.16/F-5.17/F-5.19 -> harness 7 (+ curator component suites).
- F-5.18/F-5.20 + NFR-5.7 -> config strict-parse and dependency-justification checks in CI
  verification stories.

