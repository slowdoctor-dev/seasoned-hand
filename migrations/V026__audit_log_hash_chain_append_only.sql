-- Issue #22 batch E: add tamper-evident audit_log hash-chain columns and
-- enforce append-only semantics in SQLite.
--
-- Existing rows predate the chain and keep NULL hashes. New rows written by
-- AuditLogger chain globally from the latest non-NULL row_hash, or from the
-- 64-zero genesis value when no hashed row exists yet.

ALTER TABLE audit_log ADD COLUMN prev_hash TEXT;
ALTER TABLE audit_log ADD COLUMN row_hash TEXT;

CREATE TRIGGER audit_log_no_update
BEFORE UPDATE ON audit_log
BEGIN
  SELECT RAISE(ABORT, 'audit_log is append-only');
END;

CREATE TRIGGER audit_log_no_delete
BEFORE DELETE ON audit_log
BEGIN
  SELECT RAISE(ABORT, 'audit_log is append-only');
END;
