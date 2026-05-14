-- Phase 2 / story 2.3 — Skill / playbook reservation tables.
-- refs: /specs/phase-2/architecture.md §2.12, §3 V009
-- refs: /specs/phase-2/stories/story-2.3.md
--
-- Phase 2 logic never writes to these tables (Phase 2 DEBT #6 keeps
-- this informational); they exist so Phase 3 (Curator + learning) is
-- purely logic, not a schema migration. Forward-compat principle.

CREATE TABLE skills (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT,
    title           TEXT    NOT NULL,
    summary         TEXT,
    schema_version  INTEGER NOT NULL,
    source_task_id  TEXT REFERENCES tasks(id),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE playbooks (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT,
    title           TEXT    NOT NULL,
    content_path    TEXT    NOT NULL,
    schema_version  INTEGER NOT NULL,
    source_task_id  TEXT REFERENCES tasks(id),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX idx_skills_tenant     ON skills(tenant_id);
CREATE INDEX idx_playbooks_tenant  ON playbooks(tenant_id);
