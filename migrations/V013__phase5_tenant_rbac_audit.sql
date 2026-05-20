-- Phase 5 V013 skeleton (BMAD Architect baseline)
--
-- This migration file is an architecture-aligned shape preview. Final DDL/DML,
-- backfill details, and table-rebuild mechanics are completed in implementation
-- stories under Phase 5 PM/GSD execution.
--
-- refs: specs/phase-5/architecture.md §3, ADR-014

BEGIN;

-- New org/user domain
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

-- Sharing ACL
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

-- Audit + per-user cost
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

-- Tenant-visible redacted projection
CREATE TABLE IF NOT EXISTS tenant_event_view (
  event_id INTEGER PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
  tenant_id TEXT NOT NULL,
  visibility_level TEXT NOT NULL CHECK(visibility_level IN ('viewer','user','admin')),
  redacted_data TEXT NOT NULL,
  searchable_text TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

-- NOTE: Phase-5 stories complete:
-- 1) deterministic tenant backfill for Phase 2-4 tables
-- 2) table-rebuild NOT NULL flips where required
-- 3) integrity checks + reconciliation gates
-- 4) ALTER session_search_index ADD COLUMN tenant_id TEXT NOT NULL
--    and visibility_level TEXT NOT NULL CHECK(visibility_level IN ('viewer','user','admin'))
--    for the OQ #11 (Option C) shared-index-with-strict-predicates story
--    (architecture §10). PM story must also pair-update the FTS triggers.

COMMIT;
