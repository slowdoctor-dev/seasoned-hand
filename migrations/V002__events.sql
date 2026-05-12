CREATE TABLE events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id    TEXT NOT NULL REFERENCES sessions(id),
  timestamp     INTEGER NOT NULL,
  type          TEXT NOT NULL CHECK(type IN
                  ('Message','Action','Observation','Plan',
                   'Knowledge','Datasource','Skill','Misc')),
  source        TEXT NOT NULL,
  data          TEXT NOT NULL
);
CREATE INDEX idx_events_session_time ON events(session_id, timestamp);
CREATE INDEX idx_events_type ON events(type);
