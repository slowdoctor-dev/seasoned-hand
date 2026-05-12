-- Phase 1 / story 1.9 — verifier persistence layer.
-- refs: /specs/phase-1/architecture.md §3.1 (verifications table),
-- refs: /specs/phase-1/architecture.md §3.2 (sessions.state widened).
-- refs: /specs/phase-1/stories/story-1.9.md

-- 1. New verifications table.
CREATE TABLE verifications (
  id                        TEXT PRIMARY KEY,
  session_id                TEXT NOT NULL REFERENCES sessions(id),
  triggered_at_event_id     INTEGER NOT NULL,
  trigger_kind              TEXT NOT NULL CHECK(trigger_kind IN
                              ('TaskComplete','Invalidation','CircuitBreaker')),
  trigger_detail            TEXT NOT NULL,
  verdict                   TEXT NOT NULL CHECK(verdict IN ('pass','fail')),
  reason                    TEXT NOT NULL,
  evidence_event_ids        TEXT NOT NULL,
  suggested_plan_update     TEXT,
  model_id                  TEXT NOT NULL,
  cost_cents                INTEGER NOT NULL DEFAULT 0,
  created_at                INTEGER NOT NULL
);
CREATE INDEX idx_verifications_session ON verifications(session_id, created_at);
CREATE INDEX idx_verifications_verdict ON verifications(verdict);

-- 2. Widen sessions.state CHECK to include 'VERIFYING'.
-- SQLite cannot ALTER a CHECK constraint in place, so we use the canonical
-- new-table → copy → drop-old → rename pattern. Columns and indexes
-- mirror V001__sessions.sql verbatim except the state CHECK list.
CREATE TABLE sessions_new (
  id            TEXT PRIMARY KEY,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  state         TEXT NOT NULL CHECK(state IN
                  ('IDLE','RUNNING','FINISHED','ERROR','SUSPENDED','VERIFYING')),
  project_id    TEXT,
  user_id       TEXT,
  title         TEXT,
  cost_cents    INTEGER NOT NULL DEFAULT 0,
  tool_calls    INTEGER NOT NULL DEFAULT 0,
  metadata      TEXT
);
INSERT INTO sessions_new
  SELECT id, created_at, updated_at, state, project_id, user_id, title,
         cost_cents, tool_calls, metadata
  FROM sessions;
DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;

-- Re-create every V001 index on `sessions`. The migration test
-- `migration_v004_preserves_sessions_indexes` enforces parity.
CREATE INDEX idx_sessions_state ON sessions(state);
