-- V023: Restore session_search_fts unicode61 diacritic folding.
--
-- V018 rebuilt the Phase 3 session-search FTS table to add tenant/visibility
-- columns but accidentally omitted V010's tokenizer. Rebuild the external
-- content FTS surface with the same RBAC columns plus
-- tokenize='unicode61 remove_diacritics 2', then repopulate from
-- session_search_index.

DROP TRIGGER IF EXISTS session_search_index_ai;
DROP TRIGGER IF EXISTS session_search_index_ad;
DROP TRIGGER IF EXISTS session_search_index_au;
DROP TABLE IF EXISTS session_search_fts;

CREATE VIRTUAL TABLE session_search_fts USING fts5(
  tenant_id UNINDEXED,
  visibility_level UNINDEXED,
  searchable_text,
  content='session_search_index',
  content_rowid='event_id',
  tokenize='unicode61 remove_diacritics 2'
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
