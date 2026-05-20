-- V015: Phase 5 story 5.7 — skills tenant_id NOT NULL flip (deferred from V013 schedule).
-- refs: /specs/phase-5/stories/story-5.7.md
-- refs: /specs/phase-5/architecture.md §3.4

PRAGMA foreign_keys = OFF;

UPDATE skills SET tenant_id = 'legacy-default' WHERE tenant_id IS NULL;

CREATE TABLE skills__new (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT    NOT NULL DEFAULT 'legacy-default',
    title           TEXT    NOT NULL,
    summary         TEXT,
    schema_version  INTEGER NOT NULL,
    source_task_id  TEXT REFERENCES tasks(id),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

INSERT INTO skills__new (
    id, tenant_id, title, summary, schema_version, source_task_id, created_at, updated_at
)
SELECT
    id, tenant_id, title, summary, schema_version, source_task_id, created_at, updated_at
FROM skills;

DROP TABLE skills;
ALTER TABLE skills__new RENAME TO skills;
CREATE INDEX idx_skills_tenant ON skills(tenant_id);

PRAGMA foreign_keys = ON;

