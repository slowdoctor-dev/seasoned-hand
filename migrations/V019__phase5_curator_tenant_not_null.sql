-- Phase 5 / story 5.17 — curator tenant boundaries hardening.
-- refs: /specs/phase-5/stories/story-5.17.md (F-5.14)

PRAGMA foreign_keys=OFF;

DROP TRIGGER IF EXISTS curator_search_index_ai;
DROP TRIGGER IF EXISTS curator_search_index_ad;
DROP TRIGGER IF EXISTS curator_search_index_au;
DROP TABLE IF EXISTS curator_search_fts;

ALTER TABLE curator_decisions RENAME TO curator_decisions_old;
CREATE TABLE curator_decisions (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT 'legacy-default',
  project_id TEXT NOT NULL,
  cycle_id TEXT NOT NULL,
  decision_type TEXT NOT NULL CHECK(decision_type IN (
    'merge','keep','archive','restore','conflict_raise','retrospective','recommendation','knowledge_write','datasource_write'
  )),
  subject_kind TEXT NOT NULL CHECK(subject_kind IN ('playbook','revision','conflict','retrospective','pattern','knowledge','datasource')),
  subject_id TEXT NOT NULL,
  confidence REAL,
  rationale_json TEXT NOT NULL,
  evidence_json TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('applied','queued_review','rejected','suppressed','error')),
  failure_category TEXT,
  created_at INTEGER NOT NULL
);
INSERT INTO curator_decisions
SELECT id, COALESCE(NULLIF(tenant_id, ''), 'legacy-default'), project_id, cycle_id, decision_type,
       subject_kind, subject_id, confidence, rationale_json, evidence_json, status, failure_category, created_at
FROM curator_decisions_old;
DROP TABLE curator_decisions_old;
CREATE INDEX idx_curator_decisions_project_time ON curator_decisions(project_id, created_at DESC);
CREATE INDEX idx_curator_decisions_cycle ON curator_decisions(cycle_id);

ALTER TABLE curator_review_queue RENAME TO curator_review_queue_old;
CREATE TABLE curator_review_queue (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT 'legacy-default',
  decision_id TEXT NOT NULL,
  project_id TEXT NOT NULL,
  queue_reason TEXT NOT NULL,
  severity TEXT NOT NULL CHECK(severity IN ('high','medium','low')),
  state TEXT NOT NULL CHECK(state IN ('pending','approved','rejected','suppressed')),
  reviewer TEXT,
  reviewer_note TEXT,
  resolved_at INTEGER,
  created_at INTEGER NOT NULL,
  FOREIGN KEY(decision_id) REFERENCES curator_decisions(id)
);
INSERT INTO curator_review_queue
SELECT id, COALESCE(NULLIF(tenant_id, ''), 'legacy-default'), decision_id, project_id, queue_reason,
       severity, state, reviewer, reviewer_note, resolved_at, created_at
FROM curator_review_queue_old;
DROP TABLE curator_review_queue_old;
CREATE INDEX idx_curator_review_pending ON curator_review_queue(project_id, state, created_at DESC);

ALTER TABLE sop_conflicts RENAME TO sop_conflicts_old;
CREATE TABLE sop_conflicts (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT 'legacy-default',
  project_id TEXT NOT NULL,
  left_revision_id TEXT NOT NULL,
  right_revision_id TEXT NOT NULL,
  structural_score REAL NOT NULL,
  semantic_score REAL NOT NULL,
  severity TEXT NOT NULL CHECK(severity IN ('low','medium','high')),
  status TEXT NOT NULL CHECK(status IN ('open','muted','resolved')),
  evidence_json TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
INSERT INTO sop_conflicts
SELECT id, COALESCE(NULLIF(tenant_id, ''), 'legacy-default'), project_id, left_revision_id,
       right_revision_id, structural_score, semantic_score, severity, status, evidence_json, created_at
FROM sop_conflicts_old;
DROP TABLE sop_conflicts_old;
CREATE INDEX idx_sop_conflicts_project_status ON sop_conflicts(project_id, status, created_at DESC);

ALTER TABLE knowledge_items RENAME TO knowledge_items_old;
CREATE TABLE knowledge_items (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT 'legacy-default',
  project_id TEXT NOT NULL,
  revision_id TEXT,
  source_task_id TEXT,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  confidence REAL,
  evidence_json TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
INSERT INTO knowledge_items
SELECT id, COALESCE(NULLIF(tenant_id, ''), 'legacy-default'), project_id, revision_id,
       source_task_id, key, value, confidence, evidence_json, created_at
FROM knowledge_items_old;
DROP TABLE knowledge_items_old;
CREATE INDEX idx_knowledge_items_project_key ON knowledge_items(project_id, key);

ALTER TABLE datasource_items RENAME TO datasource_items_old;
CREATE TABLE datasource_items (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT 'legacy-default',
  project_id TEXT NOT NULL,
  revision_id TEXT,
  source_task_id TEXT,
  source_type TEXT NOT NULL,
  source_ref TEXT NOT NULL,
  trust_level TEXT NOT NULL CHECK(trust_level IN ('l0','l1','l2')),
  confidence REAL,
  evidence_json TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
INSERT INTO datasource_items
SELECT id, COALESCE(NULLIF(tenant_id, ''), 'legacy-default'), project_id, revision_id,
       source_task_id, source_type, source_ref, trust_level, confidence, evidence_json, created_at
FROM datasource_items_old;
DROP TABLE datasource_items_old;
CREATE INDEX idx_datasource_items_project_type ON datasource_items(project_id, source_type, created_at DESC);

ALTER TABLE weekly_retrospectives RENAME TO weekly_retrospectives_old;
CREATE TABLE weekly_retrospectives (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT 'legacy-default',
  project_id TEXT NOT NULL,
  week_start INTEGER NOT NULL,
  week_end INTEGER NOT NULL,
  content TEXT NOT NULL,
  citation_coverage REAL NOT NULL,
  generation_status TEXT NOT NULL CHECK(generation_status IN ('success','refused','error')),
  created_at INTEGER NOT NULL,
  UNIQUE(project_id, week_start, week_end)
);
INSERT INTO weekly_retrospectives
SELECT id, COALESCE(NULLIF(tenant_id, ''), 'legacy-default'), project_id, week_start,
       week_end, content, citation_coverage, generation_status, created_at
FROM weekly_retrospectives_old;
DROP TABLE weekly_retrospectives_old;
CREATE INDEX idx_weekly_retrospectives_project_week ON weekly_retrospectives(project_id, week_end DESC);

ALTER TABLE retrospective_citations RENAME TO retrospective_citations_old;
CREATE TABLE retrospective_citations (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT 'legacy-default',
  retrospective_id TEXT NOT NULL,
  claim_index INTEGER NOT NULL,
  citation_kind TEXT NOT NULL CHECK(citation_kind IN ('event','decision','conflict','task')),
  citation_ref TEXT NOT NULL,
  snippet TEXT,
  FOREIGN KEY(retrospective_id) REFERENCES weekly_retrospectives(id)
);
INSERT INTO retrospective_citations
SELECT id, COALESCE(NULLIF(tenant_id, ''), 'legacy-default'), retrospective_id,
       claim_index, citation_kind, citation_ref, snippet
FROM retrospective_citations_old;
DROP TABLE retrospective_citations_old;
CREATE INDEX idx_retrospective_citations_retrospective ON retrospective_citations(retrospective_id, claim_index);

ALTER TABLE curator_search_index RENAME TO curator_search_index_old;
CREATE TABLE curator_search_index (
  row_id INTEGER PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT 'legacy-default',
  project_id TEXT NOT NULL,
  source_type TEXT NOT NULL,
  source_id TEXT NOT NULL,
  searchable_text TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
INSERT INTO curator_search_index
SELECT row_id, COALESCE(NULLIF(tenant_id, ''), 'legacy-default'), project_id,
       source_type, source_id, searchable_text, created_at
FROM curator_search_index_old;
DROP TABLE curator_search_index_old;

ALTER TABLE curator_decisions_summary RENAME TO curator_decisions_summary_old;
CREATE TABLE curator_decisions_summary (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT 'legacy-default',
  project_id TEXT NOT NULL,
  week_start INTEGER NOT NULL,
  week_end INTEGER NOT NULL,
  decision_type TEXT NOT NULL,
  decision_count INTEGER NOT NULL,
  mean_confidence REAL,
  created_at INTEGER NOT NULL,
  UNIQUE(project_id, week_start, week_end, decision_type)
);
INSERT INTO curator_decisions_summary
SELECT id, COALESCE(NULLIF(tenant_id, ''), 'legacy-default'), project_id, week_start,
       week_end, decision_type, decision_count, mean_confidence, created_at
FROM curator_decisions_summary_old;
DROP TABLE curator_decisions_summary_old;
CREATE INDEX idx_curator_decisions_summary_project_week
  ON curator_decisions_summary(project_id, week_end DESC);

CREATE VIRTUAL TABLE curator_search_fts USING fts5(
  searchable_text,
  content='curator_search_index',
  content_rowid='row_id',
  tokenize='porter unicode61'
);

CREATE TRIGGER curator_search_index_ai AFTER INSERT ON curator_search_index BEGIN
  INSERT INTO curator_search_fts(rowid, searchable_text)
  VALUES (new.row_id, new.searchable_text);
END;

CREATE TRIGGER curator_search_index_ad AFTER DELETE ON curator_search_index BEGIN
  INSERT INTO curator_search_fts(curator_search_fts, rowid, searchable_text)
  VALUES ('delete', old.row_id, old.searchable_text);
END;

CREATE TRIGGER curator_search_index_au AFTER UPDATE ON curator_search_index BEGIN
  INSERT INTO curator_search_fts(curator_search_fts, rowid, searchable_text)
  VALUES ('delete', old.row_id, old.searchable_text);
  INSERT INTO curator_search_fts(rowid, searchable_text)
  VALUES (new.row_id, new.searchable_text);
END;

INSERT INTO curator_search_fts(curator_search_fts) VALUES ('rebuild');

PRAGMA foreign_keys=ON;
