-- V021: restore task hierarchy FK + scheduler indexes dropped by V014.
-- refs: GitHub issue #9
--
-- V006 created:
-- - tasks.parent_task_id TEXT REFERENCES tasks(id)
-- - idx_tasks_parent
-- - idx_tasks_schedule partial WHERE schedule IS NOT NULL
--
-- V014 rebuilt tasks for the Phase 5 tenant_id NOT NULL flip but carried
-- parent_task_id as plain TEXT and recreated only the project/status indexes.
-- Rebuild the current V020-era table so the final schema regains the self-FK
-- and both indexes while preserving V017 owner_user_id.

PRAGMA foreign_keys = OFF;

CREATE TABLE tasks_v021_new (
    id                       TEXT PRIMARY KEY,
    project_id               TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    tenant_id                TEXT NOT NULL DEFAULT 'legacy-default',
    title                    TEXT NOT NULL,
    brief                    TEXT,
    status                   TEXT NOT NULL CHECK (status IN
                             ('drafted','briefed','confirmed','running',
                              'paused','completed','failed','cancelled',
                              'Drafted','Briefed','Confirmed','Running',
                              'Paused','Completed','Failed','Cancelled')),
    expected_due_at          INTEGER,
    completed_at             INTEGER,
    failure_reason           TEXT,
    parent_task_id           TEXT REFERENCES tasks(id),
    schedule                 TEXT,
    skill_attached_event_id  INTEGER,
    created_at               INTEGER NOT NULL,
    updated_at               INTEGER NOT NULL,
    owner_user_id            TEXT REFERENCES users(id)
);

INSERT INTO tasks_v021_new (
    id, project_id, tenant_id, title, brief, status, expected_due_at,
    completed_at, failure_reason, parent_task_id, schedule,
    skill_attached_event_id, created_at, updated_at, owner_user_id
)
SELECT
    id, project_id, tenant_id, title, brief, status, expected_due_at,
    completed_at, failure_reason, parent_task_id, schedule,
    skill_attached_event_id, created_at, updated_at, owner_user_id
FROM tasks;

DROP TABLE tasks;
ALTER TABLE tasks_v021_new RENAME TO tasks;

CREATE INDEX idx_tasks_project_status ON tasks(project_id, status);
CREATE INDEX idx_tasks_status_due ON tasks(status, expected_due_at);
CREATE INDEX idx_tasks_tenant_status ON tasks(tenant_id, status);
CREATE INDEX idx_tasks_owner_user_id ON tasks(owner_user_id);
CREATE INDEX idx_tasks_parent ON tasks(parent_task_id);
CREATE INDEX idx_tasks_schedule ON tasks(schedule) WHERE schedule IS NOT NULL;

PRAGMA foreign_key_check;
CREATE TEMP TABLE v021_foreign_key_check (
    violation_count INTEGER NOT NULL CHECK (violation_count = 0)
);
INSERT INTO v021_foreign_key_check
SELECT COUNT(*) FROM pragma_foreign_key_check;
DROP TABLE v021_foreign_key_check;

PRAGMA foreign_keys = ON;
