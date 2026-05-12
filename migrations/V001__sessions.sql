CREATE TABLE sessions (
  id            TEXT PRIMARY KEY,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  state         TEXT NOT NULL CHECK(state IN
                  ('IDLE','RUNNING','FINISHED','ERROR','SUSPENDED')),
  project_id    TEXT,
  user_id       TEXT,
  title         TEXT,
  cost_cents    INTEGER NOT NULL DEFAULT 0,
  tool_calls    INTEGER NOT NULL DEFAULT 0,
  metadata      TEXT
);
CREATE INDEX idx_sessions_state ON sessions(state);
