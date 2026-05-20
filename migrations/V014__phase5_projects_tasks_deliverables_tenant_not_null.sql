-- Phase 5 story 5.5:
-- - Apply the deferred Step B create-copy-rename NOT NULL flips for
--   projects/tasks/deliverables (architecture §3.4 schedule).
-- - V013 already backfilled tenant_id values, so this migration is
--   schema-only (no additional data backfill).

PRAGMA foreign_keys = OFF;

CREATE TABLE projects_v014_new (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL DEFAULT 'legacy-default',
    title       TEXT NOT NULL,
    description TEXT,
    status      TEXT NOT NULL CHECK (status IN ('active','archived')),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
INSERT INTO projects_v014_new (
    id, tenant_id, title, description, status, created_at, updated_at
)
SELECT
    id, tenant_id, title, description, status, created_at, updated_at
FROM projects;
DROP TABLE projects;
ALTER TABLE projects_v014_new RENAME TO projects;
CREATE INDEX idx_projects_status ON projects(status);
CREATE INDEX idx_projects_tenant_status ON projects(tenant_id, status);

CREATE TABLE tasks_v014_new (
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
    parent_task_id           TEXT,
    schedule                 TEXT,
    skill_attached_event_id  INTEGER,
    created_at               INTEGER NOT NULL,
    updated_at               INTEGER NOT NULL
);
INSERT INTO tasks_v014_new (
    id, project_id, tenant_id, title, brief, status, expected_due_at,
    completed_at, failure_reason, parent_task_id, schedule, skill_attached_event_id,
    created_at, updated_at
)
SELECT
    id, project_id, tenant_id, title, brief, status, expected_due_at,
    completed_at, failure_reason, parent_task_id, schedule, skill_attached_event_id,
    created_at, updated_at
FROM tasks;
DROP TABLE tasks;
ALTER TABLE tasks_v014_new RENAME TO tasks;
CREATE INDEX idx_tasks_project_status ON tasks(project_id, status);
CREATE INDEX idx_tasks_status_due ON tasks(status, expected_due_at);
CREATE INDEX idx_tasks_tenant_status ON tasks(tenant_id, status);

CREATE TABLE deliverables_v014_new (
    id                      TEXT PRIMARY KEY,
    task_id                 TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    tenant_id               TEXT NOT NULL DEFAULT 'legacy-default',
    format                  TEXT NOT NULL,
    source_content_path     TEXT,
    source_content_sha256   TEXT,
    rendered_content_path   TEXT NOT NULL,
    rendered_content_sha256 TEXT NOT NULL,
    content_size            INTEGER NOT NULL CHECK(content_size >= 0),
    citations               TEXT,
    provenance_manifest     TEXT NOT NULL DEFAULT '{}',
    created_at              INTEGER NOT NULL
);
INSERT INTO deliverables_v014_new (
    id, task_id, tenant_id, format, source_content_path, source_content_sha256,
    rendered_content_path, rendered_content_sha256, content_size, citations,
    provenance_manifest, created_at
)
SELECT
    id, task_id, tenant_id, format, source_content_path, source_content_sha256,
    rendered_content_path, rendered_content_sha256, content_size, citations,
    provenance_manifest, created_at
FROM deliverables;
DROP TABLE deliverables;
ALTER TABLE deliverables_v014_new RENAME TO deliverables;
CREATE INDEX idx_deliverables_task_created ON deliverables(task_id, created_at);
CREATE INDEX idx_deliverables_tenant ON deliverables(tenant_id);

PRAGMA foreign_keys = ON;
