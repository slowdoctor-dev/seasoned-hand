CREATE TABLE plans (
  id                TEXT PRIMARY KEY,
  session_id        TEXT NOT NULL REFERENCES sessions(id),
  goal              TEXT NOT NULL,
  phases            TEXT NOT NULL,
  current_phase_id  INTEGER NOT NULL,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);
CREATE INDEX idx_plans_session ON plans(session_id);
