-- Phase 4 / story 4.2 — curator schema + revision graph + review queue.
-- refs: /specs/phase-4/stories/story-4.2.md
-- refs: /specs/phase-4/requirements.md (F-4.14, F-4.16, F-4.26, NFR-4.3)

-- 1) playbooks denormalization + active revision pointer
ALTER TABLE playbooks ADD COLUMN source_project_id TEXT;
ALTER TABLE playbooks ADD COLUMN active_revision_id TEXT;
ALTER TABLE playbooks ADD COLUMN archived_reason TEXT;
ALTER TABLE playbooks ADD COLUMN archived_at INTEGER;

CREATE INDEX idx_playbooks_project_status ON playbooks(source_project_id, status);

-- 2) revision graph
CREATE TABLE playbook_revisions (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  playbook_id TEXT NOT NULL,
  revision_no INTEGER NOT NULL,
  parent_revision_id TEXT,
  title TEXT NOT NULL,
  trigger_keywords TEXT NOT NULL DEFAULT '[]',
  content TEXT NOT NULL,
  source_task_id TEXT,
  source_project_id TEXT NOT NULL,
  author_type TEXT NOT NULL CHECK(author_type IN ('human','curator','extractor')),
  change_kind TEXT NOT NULL CHECK(change_kind IN ('extract','merge','improve','archive','restore')),
  confidence REAL,
  created_at INTEGER NOT NULL,
  superseded_at INTEGER,
  UNIQUE(playbook_id, revision_no),
  FOREIGN KEY(playbook_id) REFERENCES playbooks(id),
  FOREIGN KEY(parent_revision_id) REFERENCES playbook_revisions(id) ON DELETE SET NULL
);
CREATE INDEX idx_playbook_revisions_playbook ON playbook_revisions(playbook_id, revision_no DESC);
CREATE INDEX idx_playbook_revisions_project ON playbook_revisions(source_project_id, created_at DESC);

-- 3) revision-scoped outcomes
CREATE TABLE playbook_revision_outcomes (
  revision_id TEXT PRIMARY KEY,
  tenant_id TEXT,
  success_count INTEGER NOT NULL DEFAULT 0,
  failure_count INTEGER NOT NULL DEFAULT 0,
  decayed_success REAL NOT NULL DEFAULT 0,
  decayed_failure REAL NOT NULL DEFAULT 0,
  last_outcome_at INTEGER,
  FOREIGN KEY(revision_id) REFERENCES playbook_revisions(id)
);

-- 4) curator decision ledger
CREATE TABLE curator_decisions (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
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
CREATE INDEX idx_curator_decisions_project_time ON curator_decisions(project_id, created_at DESC);
CREATE INDEX idx_curator_decisions_cycle ON curator_decisions(cycle_id);

-- 5) review queue
CREATE TABLE curator_review_queue (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
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
CREATE INDEX idx_curator_review_pending ON curator_review_queue(project_id, state, created_at DESC);

-- 6) conflict artifacts
CREATE TABLE sop_conflicts (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
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
CREATE INDEX idx_sop_conflicts_project_status ON sop_conflicts(project_id, status, created_at DESC);

-- 7) knowledge + datasource writers
CREATE TABLE knowledge_items (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  project_id TEXT NOT NULL,
  revision_id TEXT,
  source_task_id TEXT,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  confidence REAL,
  evidence_json TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_knowledge_items_project_key ON knowledge_items(project_id, key);

CREATE TABLE datasource_items (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
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
CREATE INDEX idx_datasource_items_project_type ON datasource_items(project_id, source_type, created_at DESC);

-- 8) retrospectives + citations
CREATE TABLE weekly_retrospectives (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  project_id TEXT NOT NULL,
  week_start INTEGER NOT NULL,
  week_end INTEGER NOT NULL,
  content TEXT NOT NULL,
  citation_coverage REAL NOT NULL,
  generation_status TEXT NOT NULL CHECK(generation_status IN ('success','refused','error')),
  created_at INTEGER NOT NULL,
  UNIQUE(project_id, week_start, week_end)
);
CREATE INDEX idx_weekly_retrospectives_project_week ON weekly_retrospectives(project_id, week_end DESC);

CREATE TABLE retrospective_citations (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  retrospective_id TEXT NOT NULL,
  claim_index INTEGER NOT NULL,
  citation_kind TEXT NOT NULL CHECK(citation_kind IN ('event','decision','conflict','task')),
  citation_ref TEXT NOT NULL,
  snippet TEXT,
  FOREIGN KEY(retrospective_id) REFERENCES weekly_retrospectives(id)
);
CREATE INDEX idx_retrospective_citations_retrospective ON retrospective_citations(retrospective_id, claim_index);

-- 9) search index + FTS
CREATE TABLE curator_search_index (
  row_id INTEGER PRIMARY KEY,
  tenant_id TEXT,
  project_id TEXT NOT NULL,
  source_type TEXT NOT NULL,
  source_id TEXT NOT NULL,
  searchable_text TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

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

-- 10) backfill from V010 rows (idempotent in one-time migration context)
UPDATE playbooks
SET source_project_id = (
  SELECT tasks.project_id FROM tasks WHERE tasks.id = playbooks.source_task_id
)
WHERE source_task_id IS NOT NULL;

UPDATE playbooks
SET source_project_id = ''
WHERE source_project_id IS NULL;

INSERT INTO playbook_revisions (
  id,
  tenant_id,
  playbook_id,
  revision_no,
  parent_revision_id,
  title,
  trigger_keywords,
  content,
  source_task_id,
  source_project_id,
  author_type,
  change_kind,
  confidence,
  created_at,
  superseded_at
)
SELECT
  'rev-' || p.id || '-1',
  p.tenant_id,
  p.id,
  1,
  NULL,
  p.title,
  p.trigger_keywords,
  p.content,
  p.source_task_id,
  p.source_project_id,
  'extractor',
  'extract',
  1.0,
  COALESCE(p.updated_at, p.created_at, CAST(unixepoch('subsec') * 1000000 AS INTEGER)),
  NULL
FROM playbooks p;

UPDATE playbooks
SET active_revision_id = 'rev-' || id || '-1';

INSERT INTO playbook_revision_outcomes (
  revision_id,
  tenant_id,
  success_count,
  failure_count,
  decayed_success,
  decayed_failure,
  last_outcome_at
)
SELECT
  'rev-' || p.id || '-1',
  p.tenant_id,
  p.success_count,
  p.failure_count,
  CAST(p.success_count AS REAL),
  CAST(p.failure_count AS REAL),
  COALESCE(p.updated_at, p.created_at, CAST(unixepoch('subsec') * 1000000 AS INTEGER))
FROM playbooks p;

-- Rebuild playbooks FTS once after backfill so newly copied `content` and
-- trigger fields are fully indexed.
INSERT INTO playbooks_fts(playbooks_fts) VALUES ('rebuild');
