-- V016: Phase 5 story 5.8 — playbooks tenant_id NOT NULL flip (deferred from V013 schedule).
-- refs: /specs/phase-5/stories/story-5.8.md
-- refs: /specs/phase-5/architecture.md §3.4
--
-- The playbooks table has accumulated columns across V009 (base shape), V010 (Phase 3
-- learning artifacts: trigger_keywords, content, success_count, failure_count,
-- avg_duration_ms, avg_tool_calls, status, version), and V011 (Phase 4 curator:
-- source_project_id, active_revision_id, archived_reason, archived_at). All 13 columns
-- carry forward through the rebuild below.

PRAGMA foreign_keys = OFF;

-- Backfill is already complete from V013; this just guards against any drift.
UPDATE playbooks SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

CREATE TABLE playbooks__new (
    id                  TEXT    PRIMARY KEY,
    tenant_id           TEXT    NOT NULL DEFAULT 'legacy-default',
    title               TEXT    NOT NULL,
    content_path        TEXT    NOT NULL,
    schema_version      INTEGER NOT NULL,
    source_task_id      TEXT REFERENCES tasks(id),
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    trigger_keywords    TEXT    NOT NULL DEFAULT '[]',
    content             TEXT    NOT NULL DEFAULT '',
    success_count       INTEGER NOT NULL DEFAULT 0,
    failure_count       INTEGER NOT NULL DEFAULT 0,
    avg_duration_ms     INTEGER,
    avg_tool_calls      INTEGER,
    status              TEXT    NOT NULL DEFAULT 'active',
    version             INTEGER NOT NULL DEFAULT 1,
    source_project_id   TEXT,
    active_revision_id  TEXT,
    archived_reason     TEXT,
    archived_at         INTEGER
);

INSERT INTO playbooks__new (
    id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at,
    trigger_keywords, content, success_count, failure_count, avg_duration_ms, avg_tool_calls,
    status, version, source_project_id, active_revision_id, archived_reason, archived_at
)
SELECT
    id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at,
    trigger_keywords, content, success_count, failure_count, avg_duration_ms, avg_tool_calls,
    status, version, source_project_id, active_revision_id, archived_reason, archived_at
FROM playbooks;

DROP TABLE playbooks;
ALTER TABLE playbooks__new RENAME TO playbooks;

-- Recreate indexes (V009 base + V010 + V011 additions).
CREATE INDEX idx_playbooks_tenant            ON playbooks(tenant_id);
CREATE INDEX idx_playbooks_source_task_id    ON playbooks(source_task_id);
CREATE INDEX idx_playbooks_source_project_id ON playbooks(source_project_id);

-- Re-create the V010 FTS maintenance triggers — they were dropped along
-- with the old `playbooks` table during the table-rebuild above. Without
-- them, INSERTs / UPDATEs / DELETEs on the new `playbooks` won't keep
-- `playbooks_fts` in sync and the matcher's production_match path returns
-- zero rows for content that was only indexed via these triggers.
CREATE TRIGGER playbooks_ai AFTER INSERT ON playbooks BEGIN
  INSERT INTO playbooks_fts(rowid, title, trigger_keywords, content)
  VALUES (new.rowid, new.title, new.trigger_keywords, new.content);
END;

CREATE TRIGGER playbooks_ad AFTER DELETE ON playbooks BEGIN
  INSERT INTO playbooks_fts(playbooks_fts, rowid, title, trigger_keywords, content)
  VALUES ('delete', old.rowid, old.title, old.trigger_keywords, old.content);
END;

CREATE TRIGGER playbooks_au AFTER UPDATE ON playbooks BEGIN
  INSERT INTO playbooks_fts(playbooks_fts, rowid, title, trigger_keywords, content)
  VALUES ('delete', old.rowid, old.title, old.trigger_keywords, old.content);
  INSERT INTO playbooks_fts(rowid, title, trigger_keywords, content)
  VALUES (new.rowid, new.title, new.trigger_keywords, new.content);
END;

-- Rebuild the FTS index from the post-rebuild playbooks rows (the rebuild
-- runs after triggers are in place so future writes keep it current).
INSERT INTO playbooks_fts(playbooks_fts) VALUES ('rebuild');

PRAGMA foreign_keys = ON;
