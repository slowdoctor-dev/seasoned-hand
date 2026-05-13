-- Phase 2 / story 2.2 — Project / Task baseline + tenancy + skill slots.
-- refs: /specs/phase-2/architecture.md §2.1, §3 V006
-- refs: /specs/phase-2/stories/story-2.2.md
--
-- tenant_id is nullable in Phase 2; the Phase 5 flip to NOT NULL is a
-- default change, not a schema change (architecture §3 V006).
-- parent_task_id / schedule / skill_attached_event_id on tasks are slot
-- reservations populated by Phase 3 (learning) / Phase 5 (workflow)
-- without requiring a follow-up migration.

CREATE TABLE projects (
    id          TEXT    PRIMARY KEY,
    tenant_id   TEXT,
    title       TEXT    NOT NULL,
    description TEXT,
    status      TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE tasks (
    id                        TEXT    PRIMARY KEY,
    project_id                TEXT    NOT NULL REFERENCES projects(id),
    tenant_id                 TEXT,
    title                     TEXT    NOT NULL,
    brief                     TEXT,
    status                    TEXT    NOT NULL,
    expected_due_at           INTEGER,
    completed_at              INTEGER,
    failure_reason            TEXT,
    parent_task_id            TEXT REFERENCES tasks(id),
    schedule                  TEXT,
    skill_attached_event_id   INTEGER,
    created_at                INTEGER NOT NULL,
    updated_at                INTEGER NOT NULL
);

ALTER TABLE sessions ADD COLUMN task_id TEXT REFERENCES tasks(id);

CREATE INDEX idx_projects_tenant_status ON projects(tenant_id, status);
CREATE INDEX idx_tasks_project_status   ON tasks(project_id, status);
CREATE INDEX idx_tasks_parent           ON tasks(parent_task_id);
CREATE INDEX idx_tasks_schedule         ON tasks(schedule) WHERE schedule IS NOT NULL;
CREATE INDEX idx_sessions_task_id       ON sessions(task_id);
