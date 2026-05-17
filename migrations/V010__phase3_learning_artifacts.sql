-- Phase 3 / story 3.2 — learning artifacts schema + search indexes.
-- refs: /specs/phase-3/stories/story-3.2.md
-- refs: /specs/phase-3/requirements.md (F-3.5, F-3.10, F-3.16, F-3.19, F-3.21, NFR-3.6)

-- Extend V009 playbooks table to align with Phase 3 architecture.
ALTER TABLE playbooks ADD COLUMN trigger_keywords TEXT NOT NULL DEFAULT '[]';
ALTER TABLE playbooks ADD COLUMN content TEXT NOT NULL DEFAULT '';
ALTER TABLE playbooks ADD COLUMN success_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playbooks ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playbooks ADD COLUMN avg_duration_ms INTEGER;
ALTER TABLE playbooks ADD COLUMN avg_tool_calls INTEGER;
ALTER TABLE playbooks ADD COLUMN status TEXT NOT NULL DEFAULT 'active';
ALTER TABLE playbooks ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

CREATE VIRTUAL TABLE playbooks_fts USING fts5(
  title,
  trigger_keywords,
  content,
  content='playbooks',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);

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

-- Ensure FTS index includes pre-V010 rows.
INSERT INTO playbooks_fts(playbooks_fts) VALUES ('rebuild');

CREATE TABLE sops (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  content TEXT NOT NULL,
  version INTEGER NOT NULL,
  enforced BOOLEAN NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX idx_sops_title ON sops(title);

CREATE TABLE glossary (
  id TEXT PRIMARY KEY,
  term TEXT NOT NULL UNIQUE,
  definition TEXT NOT NULL,
  category TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE session_search_index (
  event_id INTEGER PRIMARY KEY,
  session_id TEXT NOT NULL,
  timestamp INTEGER NOT NULL,
  event_type TEXT NOT NULL CHECK(event_type IN (
    'Message',
    'Action',
    'Observation',
    'Plan',
    'Knowledge',
    'Datasource',
    'Skill',
    'Misc'
  )),
  source TEXT NOT NULL,
  searchable_text TEXT NOT NULL
);

CREATE INDEX idx_session_search_session_time ON session_search_index(session_id, timestamp);
CREATE INDEX idx_session_search_type ON session_search_index(event_type);

CREATE VIRTUAL TABLE session_search_fts USING fts5(
  searchable_text,
  content='session_search_index',
  content_rowid='event_id',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER session_search_index_ai AFTER INSERT ON session_search_index BEGIN
  INSERT INTO session_search_fts(rowid, searchable_text)
  VALUES (new.event_id, new.searchable_text);
END;

CREATE TRIGGER session_search_index_ad AFTER DELETE ON session_search_index BEGIN
  INSERT INTO session_search_fts(session_search_fts, rowid, searchable_text)
  VALUES ('delete', old.event_id, old.searchable_text);
END;

CREATE TRIGGER session_search_index_au AFTER UPDATE ON session_search_index BEGIN
  INSERT INTO session_search_fts(session_search_fts, rowid, searchable_text)
  VALUES ('delete', old.event_id, old.searchable_text);
  INSERT INTO session_search_fts(rowid, searchable_text)
  VALUES (new.event_id, new.searchable_text);
END;
