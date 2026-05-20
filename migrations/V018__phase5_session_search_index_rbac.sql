-- V018: Phase 5 story 5.15 session-search RBAC/visibility FTS sync
--
-- Recreate session_search_fts + triggers so the virtual table mirrors
-- `tenant_id` + `visibility_level` columns added to session_search_index
-- in V013. Then rebuild from content table.

DROP TRIGGER IF EXISTS session_search_index_ai;
DROP TRIGGER IF EXISTS session_search_index_ad;
DROP TRIGGER IF EXISTS session_search_index_au;
DROP TABLE IF EXISTS session_search_fts;

CREATE VIRTUAL TABLE session_search_fts USING fts5(
  tenant_id UNINDEXED,
  visibility_level UNINDEXED,
  searchable_text,
  content='session_search_index',
  content_rowid='event_id'
);

CREATE TRIGGER session_search_index_ai AFTER INSERT ON session_search_index BEGIN
  INSERT INTO session_search_fts(rowid, tenant_id, visibility_level, searchable_text)
  VALUES (new.event_id, new.tenant_id, new.visibility_level, new.searchable_text);
END;

CREATE TRIGGER session_search_index_ad AFTER DELETE ON session_search_index BEGIN
  INSERT INTO session_search_fts(session_search_fts, rowid, tenant_id, visibility_level, searchable_text)
  VALUES ('delete', old.event_id, old.tenant_id, old.visibility_level, old.searchable_text);
END;

CREATE TRIGGER session_search_index_au AFTER UPDATE ON session_search_index BEGIN
  INSERT INTO session_search_fts(session_search_fts, rowid, tenant_id, visibility_level, searchable_text)
  VALUES ('delete', old.event_id, old.tenant_id, old.visibility_level, old.searchable_text);
  INSERT INTO session_search_fts(rowid, tenant_id, visibility_level, searchable_text)
  VALUES (new.event_id, new.tenant_id, new.visibility_level, new.searchable_text);
END;

INSERT INTO session_search_fts(session_search_fts) VALUES('rebuild');
