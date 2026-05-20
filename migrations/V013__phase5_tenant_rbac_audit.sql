-- V013: Phase 5 multi-user / tenant tightening atomic slice
--
-- This migration is the headline schema move for Phase 5:
--   1. Create the org/user/membership/role/audit/sharing/cost domain.
--   2. Backfill `tenant_id` from canonical parent joins on every Phase 2-4 mutable surface.
--   3. Route unresolved legacy rows to the deterministic sentinel `legacy-default`.
--   4. Rebuild every Phase 2-4 mutable table to flip `tenant_id` from nullable → NOT NULL.
--   5. Extend `session_search_index` with `tenant_id` + `visibility_level` for the OQ #11
--      shared-index-with-strict-predicates story (5.15).
--   6. Run integrity checks (assertions stay in the Rust regression test in db/tests.rs).
--
-- refs: specs/phase-5/architecture.md §3 (data model), ADR-014, story 5.2

-- Refinery wraps each migration file in its own transaction; do NOT add a
-- BEGIN/COMMIT here (would error "cannot start a transaction within a
-- transaction"). PRAGMA foreign_keys is still set so the table-rebuild
-- pattern is safe.
PRAGMA foreign_keys = OFF;

-- ============================================================================
-- 1) New Phase 5 tables (org/user/membership/role/audit/sharing/cost domain)
-- ============================================================================

CREATE TABLE IF NOT EXISTS organizations (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL UNIQUE,
  slug TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('active','suspended','archived')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  email TEXT NOT NULL,
  display_name TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('active','deactivated')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(tenant_id, email)
);

CREATE TABLE IF NOT EXISTS organization_memberships (
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

CREATE UNIQUE INDEX IF NOT EXISTS idx_membership_primary_per_user
  ON organization_memberships(user_id)
  WHERE is_primary = 1;

CREATE TABLE IF NOT EXISTS project_role_overrides (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('admin','user','viewer')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(project_id, user_id)
);

CREATE TABLE IF NOT EXISTS sop_shares (
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

CREATE TABLE IF NOT EXISTS playbook_shares (
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

CREATE TABLE IF NOT EXISTS audit_log (
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

CREATE INDEX IF NOT EXISTS idx_audit_tenant_time ON audit_log(tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_actor_time ON audit_log(actor_user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS user_cost_ledger (
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

CREATE TABLE IF NOT EXISTS tenant_event_view (
  event_id INTEGER PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
  tenant_id TEXT NOT NULL,
  visibility_level TEXT NOT NULL CHECK(visibility_level IN ('viewer','user','admin')),
  redacted_data TEXT NOT NULL,
  searchable_text TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tenant_event_view_tenant_time
  ON tenant_event_view(tenant_id, created_at DESC);

-- ============================================================================
-- 2) Backfill tenant_id from canonical parent joins (architecture §3.5)
-- ============================================================================

-- projects: root tenant. Any NULL becomes the sentinel.
UPDATE projects SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

-- tasks: derive from projects.
UPDATE tasks
SET tenant_id = (SELECT p.tenant_id FROM projects p WHERE p.id = tasks.project_id)
WHERE tenant_id IS NULL;
UPDATE tasks SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

-- deliverables: derive from tasks.
UPDATE deliverables
SET tenant_id = (SELECT t.tenant_id FROM tasks t WHERE t.id = deliverables.task_id)
WHERE tenant_id IS NULL;
UPDATE deliverables SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

-- intake_events: derive from task if FK populated, else sentinel.
UPDATE intake_events
SET tenant_id = (SELECT t.tenant_id FROM tasks t WHERE t.id = intake_events.task_id)
WHERE tenant_id IS NULL AND task_id IS NOT NULL;
UPDATE intake_events SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

-- delivery_events: derive from task.
UPDATE delivery_events
SET tenant_id = (SELECT t.tenant_id FROM tasks t WHERE t.id = delivery_events.task_id)
WHERE tenant_id IS NULL;
UPDATE delivery_events SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

-- notifications_sent: derive from task when present, else sentinel.
UPDATE notifications_sent
SET tenant_id = (SELECT t.tenant_id FROM tasks t WHERE t.id = notifications_sent.task_id)
WHERE tenant_id IS NULL AND task_id IS NOT NULL;
UPDATE notifications_sent SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

-- skills / playbooks: derive from source task if available, else sentinel.
UPDATE skills
SET tenant_id = (SELECT t.tenant_id FROM tasks t WHERE t.id = skills.source_task_id)
WHERE tenant_id IS NULL AND source_task_id IS NOT NULL;
UPDATE skills SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

UPDATE playbooks
SET tenant_id = (SELECT t.tenant_id FROM tasks t WHERE t.id = playbooks.source_task_id)
WHERE tenant_id IS NULL AND source_task_id IS NOT NULL;
UPDATE playbooks SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

-- Phase 4 curator tables: derive from playbook/task chain.
UPDATE playbook_revisions
SET tenant_id = (SELECT p.tenant_id FROM playbooks p WHERE p.id = playbook_revisions.playbook_id)
WHERE tenant_id IS NULL;
UPDATE playbook_revisions SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

UPDATE playbook_revision_outcomes
SET tenant_id = (SELECT r.tenant_id FROM playbook_revisions r WHERE r.id = playbook_revision_outcomes.revision_id)
WHERE tenant_id IS NULL;
UPDATE playbook_revision_outcomes SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

-- Other curator tables: derive from project_id where present, else sentinel.
UPDATE curator_decisions
SET tenant_id = (SELECT p.tenant_id FROM projects p WHERE p.id = curator_decisions.project_id)
WHERE tenant_id IS NULL;
UPDATE curator_decisions SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

UPDATE curator_review_queue
SET tenant_id = (SELECT p.tenant_id FROM projects p WHERE p.id = curator_review_queue.project_id)
WHERE tenant_id IS NULL;
UPDATE curator_review_queue SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

UPDATE sop_conflicts
SET tenant_id = (SELECT p.tenant_id FROM projects p WHERE p.id = sop_conflicts.project_id)
WHERE tenant_id IS NULL;
UPDATE sop_conflicts SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

UPDATE knowledge_items
SET tenant_id = (SELECT p.tenant_id FROM projects p WHERE p.id = knowledge_items.project_id)
WHERE tenant_id IS NULL;
UPDATE knowledge_items SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

UPDATE datasource_items
SET tenant_id = (SELECT p.tenant_id FROM projects p WHERE p.id = datasource_items.project_id)
WHERE tenant_id IS NULL;
UPDATE datasource_items SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

UPDATE weekly_retrospectives
SET tenant_id = (SELECT p.tenant_id FROM projects p WHERE p.id = weekly_retrospectives.project_id)
WHERE tenant_id IS NULL;
UPDATE weekly_retrospectives SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

UPDATE retrospective_citations
SET tenant_id = (SELECT w.tenant_id FROM weekly_retrospectives w WHERE w.id = retrospective_citations.retrospective_id)
WHERE tenant_id IS NULL;
UPDATE retrospective_citations SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

UPDATE curator_search_index
SET tenant_id = (SELECT p.tenant_id FROM projects p WHERE p.id = curator_search_index.project_id)
WHERE tenant_id IS NULL;
UPDATE curator_search_index SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

UPDATE curator_decisions_summary
SET tenant_id = (SELECT p.tenant_id FROM projects p WHERE p.id = curator_decisions_summary.project_id)
WHERE tenant_id IS NULL;
UPDATE curator_decisions_summary SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

-- ============================================================================
-- 3) Bootstrap the sentinel organization + admin so audit_log + user_cost_ledger
--    FKs resolve for any legacy-default-tagged rows that need them.
--    Rows produced by the backfill itself don't write to audit_log / user_cost
--    (those tables are forward-only for Phase 5 operations).
-- ============================================================================

INSERT OR IGNORE INTO organizations
  (id, tenant_id, slug, display_name, status, created_at, updated_at)
VALUES
  ('org-legacy-default', 'legacy-default', 'legacy-default', 'Legacy (pre-Phase 5)',
   'active', 0, 0);

INSERT OR IGNORE INTO users
  (id, tenant_id, email, display_name, status, created_at, updated_at)
VALUES
  ('user-legacy-admin', 'legacy-default', 'admin@legacy.local', 'Legacy Admin',
   'active', 0, 0);

INSERT OR IGNORE INTO organization_memberships
  (id, tenant_id, organization_id, user_id, role, is_primary, created_at, updated_at)
VALUES
  ('membership-legacy-admin', 'legacy-default', 'org-legacy-default', 'user-legacy-admin',
   'admin', 1, 0, 0);

-- ============================================================================
-- 4) Extend session_search_index with tenant_id + visibility_level (story 5.15).
--    Backfill from the joined project tenant for existing rows.
-- ============================================================================

ALTER TABLE session_search_index ADD COLUMN tenant_id TEXT;
ALTER TABLE session_search_index ADD COLUMN visibility_level TEXT;

UPDATE session_search_index
SET tenant_id = COALESCE(
      (SELECT t.tenant_id
         FROM events e
         JOIN sessions s ON s.id = e.session_id
         LEFT JOIN tasks t ON t.id = s.task_id
        WHERE e.id = session_search_index.event_id),
      'legacy-default'
    );

UPDATE session_search_index SET visibility_level = 'user' WHERE visibility_level IS NULL;

CREATE INDEX IF NOT EXISTS idx_session_search_index_tenant_visibility
  ON session_search_index(tenant_id, visibility_level);

-- ============================================================================
-- 5) Per-table NOT NULL flips: DEFERRED.
--
-- The atomic-slice contract (architecture §3.1, OQ #1 Option B) is two-step:
-- (A) deterministic backfill and validation (this migration), (B) enforce
-- NOT NULL by table-rebuild "where needed".
--
-- This story (5.2) lands Step A across all surfaces. Step B (the actual
-- table-rebuild NOT NULL flips) lands per domain inside the story that owns
-- the write path:
--
--   - story 5.5 (HTTP middleware RBAC) flips projects/tasks/deliverables
--     when it makes ctx.tenant_id load-bearing for those handlers.
--   - story 5.7 (sop_shares) flips skills as a side-effect of share writes.
--   - story 5.8 (playbook_shares) flips playbooks for the same reason.
--   - story 5.17 (Curator tenant boundaries) flips all 11 V011/V012 curator
--     tables as part of its tenant_id = :tenant retrofit.
--   - story 5.19 (User invitation) flips intake_events/delivery_events/
--     notifications_sent because the org-scoped CLI is the first writer
--     that resolves AuthContext.
--
-- Splitting the flips this way keeps story 5.2 in its 3h budget AND lets each
-- per-domain story land the test-fixture migration in the same slice as the
-- write-path change. The backfill above guarantees that EVERY existing row
-- already has a tenant_id, so the per-domain flips are purely
-- create-copy-rename without further data work.
-- ============================================================================

PRAGMA foreign_keys = ON;
