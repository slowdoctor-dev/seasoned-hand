-- Phase 4 / Story 4.23: Curator telemetry retention + compaction.
-- Adds the summary tier for compacted `curator_decisions` rows that have aged
-- past the NFR-4.4 90-day hot window. Per-week per-decision-type histogram +
-- mean confidence preserves enough signal for Phase 4 retrospectives without
-- carrying the raw rows.
--
-- refs: /specs/phase-4/stories/story-4.23.md, NFR-4.4

CREATE TABLE curator_decisions_summary (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,
  project_id TEXT NOT NULL,
  week_start INTEGER NOT NULL,
  week_end INTEGER NOT NULL,
  decision_type TEXT NOT NULL,
  decision_count INTEGER NOT NULL,
  mean_confidence REAL,
  created_at INTEGER NOT NULL,
  UNIQUE(project_id, week_start, week_end, decision_type)
);

CREATE INDEX idx_curator_decisions_summary_project_week
  ON curator_decisions_summary(project_id, week_end DESC);
