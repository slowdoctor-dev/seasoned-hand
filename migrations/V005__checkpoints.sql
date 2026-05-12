-- Phase 1 / story 1.13 — Checkpoint Manager persistence.
-- refs: /specs/phase-1/architecture.md §3.3 (checkpoints table)
-- refs: /specs/phase-1/stories/story-1.13.md
--
-- `rolled_back_at` + `rolled_back_by` ship now even though story 1.13b
-- owns the UPDATE path; rolling the column add into V005 avoids two
-- migrations against the same table.

CREATE TABLE checkpoints (
  id                      TEXT PRIMARY KEY,
  session_id              TEXT NOT NULL REFERENCES sessions(id),
  plan_phase_id           INTEGER NOT NULL,
  git_sha                 TEXT NOT NULL,
  label                   TEXT,
  triggered_by_event_id   INTEGER NOT NULL,
  rolled_back_at          INTEGER,
  rolled_back_by          TEXT,
  created_at              INTEGER NOT NULL
);
CREATE INDEX idx_checkpoints_session ON checkpoints(session_id, created_at);
