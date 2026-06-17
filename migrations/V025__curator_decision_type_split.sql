-- Issue #22 batch C: split curator decision_type values so archive and
-- quarantine decisions no longer collapse into generic archive/keep buckets.

PRAGMA foreign_keys=OFF;

ALTER TABLE curator_decisions RENAME TO curator_decisions_old;
CREATE TABLE curator_decisions (
  id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT 'legacy-default',
  project_id TEXT NOT NULL,
  cycle_id TEXT NOT NULL,
  decision_type TEXT NOT NULL CHECK(decision_type IN (
    'merge','keep','archive','archive_recommend','archive_apply','restore',
    'quarantine','conflict_raise','retrospective','recommendation',
    'knowledge_write','datasource_write'
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
SELECT id, tenant_id, project_id, cycle_id, decision_type,
       subject_kind, subject_id, confidence, rationale_json, evidence_json,
       status, failure_category, created_at
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
  severity TEXT NOT NULL CHECK(severity IN ('low','medium','high')),
  state TEXT NOT NULL CHECK(state IN ('pending','approved','rejected','suppressed')),
  reviewer TEXT,
  reviewer_note TEXT,
  resolved_at INTEGER,
  created_at INTEGER NOT NULL,
  FOREIGN KEY(decision_id) REFERENCES curator_decisions(id)
);
INSERT INTO curator_review_queue
SELECT id, tenant_id, decision_id, project_id, queue_reason, severity, state,
       reviewer, reviewer_note, resolved_at, created_at
FROM curator_review_queue_old;
DROP TABLE curator_review_queue_old;
CREATE INDEX idx_curator_review_pending ON curator_review_queue(project_id, state, created_at DESC);

PRAGMA foreign_key_check;
PRAGMA foreign_keys=ON;
