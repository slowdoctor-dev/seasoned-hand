-- Phase 2 / story 2.3 — Deliverables + inline provenance manifest.
-- refs: /specs/phase-2/architecture.md §2.3, §2.11, §3 V007
-- refs: /specs/phase-2/stories/story-2.3.md
--
-- provenance_manifest is a JSON TEXT column; the manifest schema is
-- defined in architecture §2.11. Phase 2 keeps the manifest inline
-- (≤100 KB budget — see Phase 2 DEBT #5 for the spill-to-file plan).
-- Citations carry event_id references inline in the deliverable
-- content; the column holds the JSON array index.

CREATE TABLE deliverables (
    id                       TEXT    PRIMARY KEY,
    task_id                  TEXT    NOT NULL REFERENCES tasks(id),
    tenant_id                TEXT,
    format                   TEXT    NOT NULL,
    source_content_path      TEXT,
    source_content_sha256    TEXT,
    rendered_content_path    TEXT    NOT NULL,
    rendered_content_sha256  TEXT    NOT NULL,
    content_size             INTEGER NOT NULL,
    citations                TEXT,
    provenance_manifest      TEXT    NOT NULL,
    created_at               INTEGER NOT NULL
);

CREATE INDEX idx_deliverables_task   ON deliverables(task_id);
CREATE INDEX idx_deliverables_tenant ON deliverables(tenant_id);
