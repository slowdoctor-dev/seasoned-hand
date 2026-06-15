-- V024: Tenant-scope SOP rows for share authorization.
--
-- V010 created `sops` as a global table, while V013+ made `sop_shares`
-- tenant-scoped. That let a tenant admin create a share row in their tenant
-- against another tenant's SOP id. Backfill existing SOPs to the Phase 5
-- legacy sentinel tenant, then require new rows to carry tenant_id.

ALTER TABLE sops ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'legacy-default';

CREATE INDEX IF NOT EXISTS idx_sops_tenant ON sops(tenant_id);
